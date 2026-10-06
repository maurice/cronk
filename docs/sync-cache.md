# Sync/cache design and validation

## Ownership

The cloneable GitLab client shares the bounded HTTP validator cache and per-project sync locks. A workspace-specific SQLite database stores list identities, resource checkpoints, resumable import boundaries and viewed detail snapshots. The filename is a SHA-256 installation/credential fingerprint, not a credential. Token rotation intentionally starts a separate cache. The database does not contain tokens or trace bodies and is not encrypted.

Machine-local navigation is persisted separately in a private `.state/` SQLite directory, without the credential fingerprint. Clearing this content cache or rotating a token does not reset navigation. See [navigation state](navigation-state.md) for migration, coalescing and shutdown contracts.

UI detail content and request ownership are keyed by `ItemKey` (project, kind, IID) within the current client/account. Tab navigation keeps independent selection, scroll, expansion and log-reading state. Switching tabs does not cancel a useful detail request or initiate a duplicate one. Changing the GitLab setup clears client-specific UI caches; obsolete request IDs cannot populate the new account's view.

## List synchronization

- Issues and MRs import independently, newest-first, with 100-entry pages.
- Each page is upserted and committed before it is delivered to the UI. An import's original start timestamp, current timestamp boundary and reconciliation generation survive restart.
- Restart resumes with an inclusive `updated_before` boundary, beginning at page one of that range rather than reusing an offset into the original moving list. Equal-timestamp items can be replayed without duplicate identities. All history is retained; there is no arbitrary history cutoff.
- Each resource advances its checkpoint only after successful complete pagination, any required disappearance checks, and a durable transaction. Another resource's failure does not roll back its successful checkpoint.
- Normal refresh uses the preceding checkpoint minus two minutes as `updated_after`, and the new sync's start as `updated_before`. The overlap reduces timestamp-boundary races; identities are upserted rather than appended.
- Full reconciliation runs after 24 hours or via the palette. Missing cached identities receive an individual GET: successful responses retain/update moved items; 403/404 removes inaccessible/missing identities; other failures leave reconciliation unfinished. A failed list request cannot be treated as an empty collection. Feature-disabled collections still require GitLab metadata verification.
- A successful remote write increments the shared request generation and invalidates detail/HTTP caches. Older GETs cannot commit detail snapshots or advance list checkpoints afterward.
- Dashboard pipeline/discussion signals refresh independently for the bounded set of recent open MRs; a parent `updated_at` delta is not assumed to cover those signals.

GitLab offset pagination is not an atomic snapshot. Unknown items moving between pages can still require later reconciliation. Accurate server timestamps and reconciliation remain important; this is not a transactional server change feed.

## Detail loading

The shell is rendered from a cached/list item immediately. The core response is published before optional label enrichment. Activity/label colors, discussions, pipeline/jobs and changes use at most four section workers per MR (two for an issue). Every completed section publishes a snapshot and saves it to SQLite. Independent sections do not wait for a slow activity/diff request. Pipeline lookup and jobs remain sequential where there is a data dependency.

Missing sections display skeletons; refresh preserves already usable sections. Empty fetched sections are distinct from unfetched sections. Optional failures retain usable cached content and expose warnings. Historic cached rate-limit warnings do not restart network backoff. Job traces remain memory-only and retain the existing per-tab reading behavior.

## Reproducible request-count comparison

Run:

```sh
just test-lib -E 'test(persistent_sync_publishes_pages_and_warm_restart_requests_only_deltas)'
```

The mock project has 1,000 issues, zero MRs, 100 entries per page, and one changed issue on refresh. The test explicitly compares the uncached all-history path with a process-restarted persistent client:

| Scenario | List HTTP requests | Content available |
| --- | ---: | --- |
| Previous all-history refresh | 11 | After the complete project collection |
| New first import | 11 | Each durable page as it arrives |
| New warm restart / incremental refresh | 2 | 1,000 cached issues before refresh; changed rows streamed |

The warm list refresh uses **82% fewer list requests** (11 → 2), without discarding older history. The test also asserts the first issue page is delivered before the second issue request and verifies that every delivered page is already durable. This is a deterministic request-budget comparison, **not a live-GitLab latency benchmark**. It excludes current-user/iteration requests and dashboard enrichment (there are no MRs in this fixture). Benefits scale with history length; daily/manual reconciliation still reads full history.

Other coverage includes interrupted imports/resumption, account isolation, restrictive Unix permissions, missing-item verification, a blocked activity request not blocking jobs/discussions/changes, failed optional refresh retaining cached content, successful-write invalidation, stale generations, shared requests across tabs, skeleton rendering and unknown-total progress.

The screenshot in `progressive-loading.png` uses fictional demo fixtures and can be regenerated with:

```sh
CRONK_CAPTURE_PROGRESS=docs/progressive-loading.png just test-integration ui -E 'test(loading_detail_sections)'
```
