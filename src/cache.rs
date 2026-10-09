//! Private, credential-isolated SQLite content cache. No credentials or trace bodies are stored.
use crate::model::{Details, ItemKey, ItemKind, WorkItem};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, sync::Mutex};

pub(crate) struct ContentCache(Mutex<Connection>);

#[derive(Debug)]
pub(crate) struct SyncPlan {
    pub after: Option<String>,
    pub before: String,
    pub started: String,
    pub full: bool,
    pub generation: i64,
}

pub(crate) fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn overlap(value: &str) -> Result<String> {
    Ok(
        (DateTime::parse_from_rfc3339(value)? - chrono::Duration::seconds(120))
            .to_rfc3339_opts(SecondsFormat::Millis, true),
    )
}

impl ContentCache {
    pub fn open(workspace: &Path, installation: &str, token: &str) -> Result<Self> {
        // A credential fingerprint also isolates different scopes for the same user.
        // Changing tokens intentionally creates a cold cache. Never persist the token.
        let fingerprint: String = Sha256::digest(format!("{installation}\0{token}"))
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let directory = workspace.with_extension("cache");
        if fs::symlink_metadata(&directory).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!("Cache directory must not be a symlink");
        }
        fs::create_dir_all(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let path = directory.join(format!("{fingerprint}.sqlite3"));
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!("Cache database must not be a symlink");
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        let connection = Connection::open(&path).context("Open content cache")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 1 {
            bail!("Content cache is newer than this version of Cronk");
        }
        connection.execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS items (
                project INTEGER NOT NULL, kind TEXT NOT NULL, iid INTEGER NOT NULL,
                body TEXT NOT NULL, seen INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(project,kind,iid));
            CREATE TABLE IF NOT EXISTS sync (
                project INTEGER NOT NULL, kind TEXT NOT NULL, checkpoint TEXT,
                full_at TEXT, started TEXT, cursor TEXT, generation INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(project,kind));
            CREATE TABLE IF NOT EXISTS details (
                project INTEGER NOT NULL, kind TEXT NOT NULL, iid INTEGER NOT NULL,
                body TEXT NOT NULL, PRIMARY KEY(project,kind,iid));
            PRAGMA user_version=1;",
        )?;
        Ok(Self(Mutex::new(connection)))
    }

    pub fn items(&self, project: u64) -> Result<Vec<WorkItem>> {
        let project = i64::try_from(project)?;
        let bodies = {
            let db = self.0.lock().unwrap_or_else(|e| e.into_inner());
            let mut statement =
                db.prepare_cached("SELECT body FROM items WHERE project=? ORDER BY kind,iid")?;
            statement
                .query_map([project], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        // JSON decoding is CPU work and must not block other cache users.
        bodies
            .into_iter()
            .map(|body| Ok(serde_json::from_str(&body)?))
            .collect()
    }

    pub fn plan(&self, project: u64, kind: ItemKind, force: bool) -> Result<SyncPlan> {
        let project = i64::try_from(project)?;
        let db = self.0.lock().unwrap_or_else(|e| e.into_inner());
        db.execute(
            "INSERT OR IGNORE INTO sync(project,kind) VALUES(?,?)",
            params![project, kind.segment()],
        )?;
        let (checkpoint, full_at, started, cursor, generation): (Option<String>,Option<String>,Option<String>,Option<String>,i64) = db.query_row(
            "SELECT checkpoint,full_at,started,cursor,generation FROM sync WHERE project=? AND kind=?",
            params![project,kind.segment()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        let now = timestamp();
        if let Some(started) = started {
            // Resume by timestamp, not a saved offset into a moving collection. Re-read
            // boundary ties inclusively; identity upserts make replay harmless.
            return Ok(SyncPlan {
                after: None,
                before: cursor.unwrap_or_else(|| started.clone()),
                started,
                full: true,
                generation,
            });
        }
        let due = full_at
            .as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .is_none_or(|t| Utc::now().signed_duration_since(t).num_hours() >= 24);
        if force || checkpoint.is_none() || due {
            let generation = generation + 1;
            db.execute(
                "UPDATE sync SET started=?,cursor=?,generation=? WHERE project=? AND kind=?",
                params![now, now, generation, project, kind.segment()],
            )?;
            Ok(SyncPlan {
                after: None,
                before: now.clone(),
                started: now,
                full: true,
                generation,
            })
        } else {
            Ok(SyncPlan {
                after: checkpoint.as_deref().map(overlap).transpose()?,
                before: now.clone(),
                started: now,
                full: false,
                generation,
            })
        }
    }

    pub fn page(
        &self,
        project: u64,
        kind: ItemKind,
        plan: &SyncPlan,
        items: &[WorkItem],
    ) -> Result<()> {
        let project = i64::try_from(project)?;
        let mut db = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let tx = db.transaction()?;
        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            let mut fresh = item.clone();
            if kind == ItemKind::MergeRequest && matches!(fresh.state.as_str(), "merged" | "closed")
            {
                let body: Option<String> = tx
                    .query_row(
                        "SELECT body FROM items WHERE project=? AND kind=? AND iid=?",
                        params![project, kind.segment(), i64::try_from(fresh.key.iid)?],
                        |row| row.get(0),
                    )
                    .optional()?;
                if let Some(body) = body {
                    let previous: WorkItem = serde_json::from_str(&body)?;
                    fresh.preserve_terminal_pipeline_metrics(&previous);
                }
            }
            rows.push((
                i64::try_from(fresh.key.iid)?,
                serde_json::to_string(&fresh)?,
            ));
        }
        {
            let mut statement = tx.prepare_cached(
                "INSERT INTO items(project,kind,iid,body,seen) VALUES(?,?,?,?,?)
                ON CONFLICT(project,kind,iid) DO UPDATE SET body=excluded.body,seen=excluded.seen",
            )?;
            for (iid, body) in rows {
                statement.execute(params![project, kind.segment(), iid, body, plan.generation])?;
            }
        }
        if plan.full
            && let Some(last) = items.last()
        {
            // A malformed timestamp must not corrupt the resumable boundary.
            DateTime::parse_from_rfc3339(&last.updated_at)?;
            tx.execute(
                "UPDATE sync SET cursor=? WHERE project=? AND kind=?",
                params![last.updated_at, project, kind.segment()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn merge_mr_pipeline_metrics(&self, item: &WorkItem) -> Result<()> {
        if item.key.kind != ItemKind::MergeRequest
            || !matches!(item.state.as_str(), "merged" | "closed")
        {
            return Ok(());
        }
        let mut db = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let tx = db.transaction()?;
        let project = i64::try_from(item.key.project)?;
        let iid = i64::try_from(item.key.iid)?;
        let body: Option<String> = tx
            .query_row(
                "SELECT body FROM items WHERE project=? AND kind=? AND iid=?",
                params![project, item.key.kind.segment(), iid],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(body) = body {
            let mut cached: WorkItem = serde_json::from_str(&body)?;
            if item.pipeline.is_some() {
                cached.pipeline.clone_from(&item.pipeline);
            }
            if item.merged_at.is_some() {
                cached.merged_at.clone_from(&item.merged_at);
            }
            if item.closed_at.is_some() {
                cached.closed_at.clone_from(&item.closed_at);
            }
            cached.pipeline_metrics_fetched = true;
            tx.execute(
                "UPDATE items SET body=? WHERE project=? AND kind=? AND iid=?",
                params![
                    serde_json::to_string(&cached)?,
                    project,
                    item.key.kind.segment(),
                    iid
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn enrich(&self, items: &[WorkItem]) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let rows = items
            .iter()
            .map(|item| {
                Ok((
                    serde_json::to_string(item)?,
                    i64::try_from(item.key.project)?,
                    item.key.kind.segment(),
                    i64::try_from(item.key.iid)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut db = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let tx = db.transaction()?;
        {
            let mut statement =
                tx.prepare_cached("UPDATE items SET body=? WHERE project=? AND kind=? AND iid=?")?;
            for (body, project, kind, iid) in rows {
                statement.execute(params![body, project, kind, iid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn missing(&self, project: u64, kind: ItemKind, plan: &SyncPlan) -> Result<Vec<ItemKey>> {
        if !plan.full {
            return Ok(Vec::new());
        }
        let db = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt =
            db.prepare("SELECT iid FROM items WHERE project=? AND kind=? AND seen<>?")?;
        Ok(stmt
            .query_map(
                params![i64::try_from(project)?, kind.segment(), plan.generation],
                |r| {
                    Ok(ItemKey {
                        project,
                        kind,
                        iid: r.get::<_, i64>(0)? as u64,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn finish(
        &self,
        project: u64,
        kind: ItemKind,
        plan: &SyncPlan,
        removed: &[ItemKey],
    ) -> Result<()> {
        let project = i64::try_from(project)?;
        let mut db = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let tx = db.transaction()?;
        {
            let mut items =
                tx.prepare_cached("DELETE FROM items WHERE project=? AND kind=? AND iid=?")?;
            let mut details =
                tx.prepare_cached("DELETE FROM details WHERE project=? AND kind=? AND iid=?")?;
            for key in removed {
                let project = i64::try_from(key.project)?;
                let iid = i64::try_from(key.iid)?;
                items.execute(params![project, key.kind.segment(), iid])?;
                details.execute(params![project, key.kind.segment(), iid])?;
            }
        }
        tx.execute("UPDATE sync SET checkpoint=?,full_at=CASE WHEN ? THEN ? ELSE full_at END,started=NULL,cursor=NULL WHERE project=? AND kind=?",
            params![plan.started,plan.full,timestamp(),project,kind.segment()])?;
        tx.commit()?;
        Ok(())
    }

    pub fn detail(&self, key: &ItemKey) -> Result<Option<Details>> {
        let body: Option<String> = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_row(
                "SELECT body FROM details WHERE project=? AND kind=? AND iid=?",
                params![
                    i64::try_from(key.project)?,
                    key.kind.segment(),
                    i64::try_from(key.iid)?
                ],
                |r| r.get(0),
            )
            .optional()?;
        body.map(|b| serde_json::from_str(&b).context("Decode cached detail"))
            .transpose()
    }

    pub fn save_detail(&self, details: &Details) -> Result<()> {
        let key = &details.item.key;
        let body = serde_json::to_string(details)?;
        self.0.lock().unwrap_or_else(|e| e.into_inner()).execute(
            "INSERT OR REPLACE INTO details(project,kind,iid,body) VALUES(?,?,?,?)",
            params![
                i64::try_from(key.project)?,
                key.kind.segment(),
                i64::try_from(key.iid)?,
                body
            ],
        )?;
        Ok(())
    }

    pub fn invalidate_details(&self) -> Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .execute("DELETE FROM details", [])?;
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .execute_batch("DELETE FROM items; DELETE FROM sync; DELETE FROM details; VACUUM;")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_fingerprint_preserves_existing_cache_filename() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace.toml");
        let _cache = ContentCache::open(&workspace, "https://example.test", "secret").unwrap();
        assert!(
            workspace
                .with_extension("cache")
                .join("67034df2a0443843676e092f86f89219c2ffc1ff8014f6af6a6c22641702e2ed.sqlite3")
                .is_file()
        );
    }

    fn item(iid: u64) -> WorkItem {
        WorkItem {
            key: ItemKey {
                project: 7,
                kind: ItemKind::Issue,
                iid,
            },
            updated_at: "2024-01-01T00:00:00Z".into(),
            ..Default::default()
        }
    }
    #[test]
    fn large_snapshot_batches_replay_enrichment_and_concurrent_reads() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ContentCache::open(
            &dir.path().join("workspace.toml"),
            "https://example.test",
            "secret",
        )
        .unwrap();
        let plan = cache.plan(7, ItemKind::Issue, false).unwrap();
        let items: Vec<_> = (1..=3_000).map(item).collect();
        let start = std::time::Instant::now();
        for page in items.chunks(100) {
            cache.page(7, ItemKind::Issue, &plan, page).unwrap();
        }
        eprintln!(
            "SQLite: 3,000 items in 30 page transactions: {:?}",
            start.elapsed()
        );
        cache
            .page(7, ItemKind::Issue, &plan, &items[..100])
            .unwrap();
        cache.finish(7, ItemKind::Issue, &plan, &[]).unwrap();
        let start = std::time::Instant::now();
        assert_eq!(cache.items(7).unwrap().len(), 3_000);
        eprintln!("SQLite: read/decode 3,000 items: {:?}", start.elapsed());
        let mut enriched = items[..100].to_vec();
        for item in &mut enriched {
            item.title = "Enriched".into();
        }
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..3 {
                    assert_eq!(cache.items(7).unwrap().len(), 3_000);
                }
            });
            scope.spawn(|| cache.enrich(&enriched).unwrap());
        });
        let snapshot = cache.items(7).unwrap();
        assert_eq!(
            snapshot.iter().filter(|i| i.title == "Enriched").count(),
            100
        );
        assert!(cache.missing(7, ItemKind::Issue, &plan).unwrap().is_empty());
        let removed: Vec<_> = items.iter().map(|i| i.key.clone()).collect();
        cache.finish(7, ItemKind::Issue, &plan, &removed).unwrap();
        assert!(cache.items(7).unwrap().is_empty());
    }

    #[test]
    fn snapshot_and_identity_reads_use_primary_key_indexes() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ContentCache::open(
            &dir.path().join("workspace.toml"),
            "https://example.test",
            "secret",
        )
        .unwrap();
        let db = cache.0.lock().unwrap();
        for sql in [
            "EXPLAIN QUERY PLAN SELECT body FROM items WHERE project=7 ORDER BY kind,iid",
            "EXPLAIN QUERY PLAN SELECT body FROM details WHERE project=7 AND kind='issues' AND iid=1",
            "EXPLAIN QUERY PLAN SELECT iid FROM items WHERE project=7 AND kind='issues' AND seen<>1",
        ] {
            let mut statement = db.prepare(sql).unwrap();
            let steps = statement
                .query_map([], |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert!(
                steps
                    .iter()
                    .any(|step| step.contains("SEARCH") && step.contains("INDEX")),
                "{steps:?}"
            );
            assert!(
                !steps
                    .iter()
                    .any(|step| step.contains("TEMP B-TREE") || step.contains("SCAN")),
                "{steps:?}"
            );
        }
    }

    #[test]
    fn malformed_page_rolls_back_items_and_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ContentCache::open(
            &dir.path().join("workspace.toml"),
            "https://example.test",
            "secret",
        )
        .unwrap();
        let plan = cache.plan(7, ItemKind::Issue, false).unwrap();
        let mut invalid = item(2);
        invalid.updated_at = "invalid".into();
        assert!(
            cache
                .page(7, ItemKind::Issue, &plan, &[item(1), invalid])
                .is_err()
        );
        assert!(cache.items(7).unwrap().is_empty());
        assert_eq!(
            cache.plan(7, ItemKind::Issue, false).unwrap().before,
            plan.before
        );
    }

    #[test]
    fn restart_resumes_then_uses_overlapping_delta() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.toml");
        let cache = ContentCache::open(&path, "https://example.test/api/v4", "secret").unwrap();
        let plan = cache.plan(7, ItemKind::Issue, false).unwrap();
        cache.page(7, ItemKind::Issue, &plan, &[item(1)]).unwrap();
        drop(cache);
        let cache = ContentCache::open(&path, "https://example.test/api/v4", "secret").unwrap();
        let resumed = cache.plan(7, ItemKind::Issue, false).unwrap();
        assert!(resumed.full);
        assert_eq!(resumed.before, "2024-01-01T00:00:00Z");
        assert_eq!(resumed.started, plan.started);
        cache
            .page(7, ItemKind::Issue, &resumed, &[item(1), item(2)])
            .unwrap();
        cache.finish(7, ItemKind::Issue, &resumed, &[]).unwrap();
        assert_eq!(cache.items(7).unwrap().len(), 2);
        let delta = cache.plan(7, ItemKind::Issue, false).unwrap();
        assert!(!delta.full);
        assert_eq!(delta.after, Some(overlap(&plan.started).unwrap()));
        assert!(cache.plan(7, ItemKind::MergeRequest, false).unwrap().full);
    }
    #[test]
    fn incomplete_delta_does_not_advance_checkpoint_and_reconciliation_is_periodic() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ContentCache::open(
            &dir.path().join("workspace.toml"),
            "https://example.test",
            "secret",
        )
        .unwrap();
        let full = cache.plan(7, ItemKind::Issue, false).unwrap();
        cache.page(7, ItemKind::Issue, &full, &[item(1)]).unwrap();
        cache.finish(7, ItemKind::Issue, &full, &[]).unwrap();
        let delta = cache.plan(7, ItemKind::Issue, false).unwrap();
        assert!(!delta.full);
        cache.page(7, ItemKind::Issue, &delta, &[item(2)]).unwrap();
        // Simulate failure after this page: replay starts at the old checkpoint.
        assert_eq!(
            cache.plan(7, ItemKind::Issue, false).unwrap().after,
            delta.after
        );
        cache
            .0
            .lock()
            .unwrap()
            .execute("UPDATE sync SET full_at='2020-01-01T00:00:00Z'", [])
            .unwrap();
        assert!(cache.plan(7, ItemKind::Issue, false).unwrap().full);
    }

    #[test]
    fn account_isolation_details_and_explicit_reconciliation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.toml");
        let cache = ContentCache::open(&path, "https://example.test", "token-a").unwrap();
        let p = cache.plan(7, ItemKind::Issue, false).unwrap();
        cache
            .page(7, ItemKind::Issue, &p, &[item(1), item(2)])
            .unwrap();
        cache.finish(7, ItemKind::Issue, &p, &[]).unwrap();
        cache
            .save_detail(&Details {
                item: item(2),
                ..Default::default()
            })
            .unwrap();
        assert!(cache.detail(&item(2).key).unwrap().is_some());
        let p = cache.plan(7, ItemKind::Issue, true).unwrap();
        cache.page(7, ItemKind::Issue, &p, &[item(1)]).unwrap();
        assert_eq!(
            cache.missing(7, ItemKind::Issue, &p).unwrap(),
            vec![item(2).key]
        );
        // A failed collection doesn't finish and cannot delete anything.
        assert_eq!(cache.items(7).unwrap().len(), 2);
        cache
            .finish(7, ItemKind::Issue, &p, &[item(2).key])
            .unwrap();
        assert!(cache.detail(&item(2).key).unwrap().is_none());
        assert!(
            ContentCache::open(&path, "https://example.test", "token-b")
                .unwrap()
                .items(7)
                .unwrap()
                .is_empty()
        );
        cache.clear().unwrap();
        assert!(cache.items(7).unwrap().is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for entry in fs::read_dir(path.with_extension("cache")).unwrap() {
                assert_eq!(
                    entry.unwrap().metadata().unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }
}
