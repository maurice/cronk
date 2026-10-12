use super::*;
use crate::{cache::ContentCache, config::SavedView, demo, model::ItemKind};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
use tempfile::tempdir;

fn config() -> Config {
    let route = demo::items()
        .into_iter()
        .find(|i| i.key.kind == ItemKind::Issue)
        .unwrap()
        .key;
    Config {
        onboarding: false,
        projects: demo::projects(),
        active_tab: 2,
        route: Some(route.clone()),
        section: Some(0),
        selections: BTreeMap::from([("2".into(), 1)]),
        tab_states: BTreeMap::from([(
            "2".into(),
            TabState {
                route: Some(route),
                section: Some(0),
                field: 1,
                content_offset: 8,
                list_offset: 4,
                expanded: [91].into(),
                collapsed: [92].into(),
                ..TabState::default()
            },
        )]),
        ..Config::default()
    }
}

fn snapshot(index: usize) -> Snapshot {
    Snapshot {
        namespace: String::new(),
        active: index.to_string(),
        tabs: BTreeMap::new(),
    }
}

fn force(writer: &Writer) {
    let (mutex, wake) = &*writer.shared;
    mutex.lock().unwrap().force = true;
    wake.notify_all();
}

#[test]
fn fake_clock_continuous_activity_has_a_bounded_first_dirty_deadline() {
    let start = Instant::now();
    let mut mailbox = Mailbox::default();
    let mut commits = Vec::new();
    for n in 0..10_000 {
        let now = start + Duration::from_millis(n as u64);
        mailbox.submit(snapshot(n), now);
        if mailbox.due(now, INTERVAL) {
            commits.push(mailbox.pending.take().unwrap().1.active);
            mailbox.dirty_at = None;
        }
    }
    assert_eq!(commits, ["2500", "5001", "7502"]);
    assert_eq!(mailbox.pending.as_ref().unwrap().1.active, "9999");
    assert_eq!(mailbox.submitted, 10_000);
    assert!(!mailbox.due(start + Duration::from_millis(10_001), INTERVAL));
    assert!(mailbox.due(start + Duration::from_millis(10_003), INTERVAL));
}

#[test]
fn queue_pressure_does_not_block_submit_and_only_keeps_latest_snapshot() {
    let (entered_tx, entered_rx) = mpsc::sync_channel(2);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let writes = Arc::new(Mutex::new(Vec::new()));
    let recorded = writes.clone();
    let writer = Writer::start(
        move |snapshot| {
            recorded.lock().unwrap().push(snapshot.active.clone());
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        },
        INTERVAL,
    )
    .unwrap();
    writer.submit(snapshot(0));
    force(&writer);
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    // The database writer is deliberately blocked. These calls must still finish.
    for n in 1..10_000 {
        writer.submit(snapshot(n));
    }
    {
        let mailbox = writer.shared.0.lock().unwrap();
        assert_eq!(mailbox.pending.as_ref().unwrap().1.active, "9999");
    }
    release_tx.send(()).unwrap();
    force(&writer);
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    release_tx.send(()).unwrap();
    writer.flush().unwrap();
    drop(writer);
    assert_eq!(*writes.lock().unwrap(), ["0", "9999"]);
}

#[test]
fn flush_and_drop_commit_latest_once_and_join_the_worker() {
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let writer = Writer::start(
        move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
        INTERVAL,
    )
    .unwrap();
    for n in 0..1000 {
        writer.submit(snapshot(n));
    }
    writer.flush().unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    // Empty flush must not make the next input bypass coalescing.
    writer.flush().unwrap();
    assert!(!writer.shared.0.lock().unwrap().force);
    writer.submit(snapshot(1001));
    drop(writer);
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[test]
fn failed_flush_retains_latest_for_retry_and_drop_finishes_without_retry_loop() {
    let fail = Arc::new(AtomicBool::new(true));
    let failing = fail.clone();
    let writes = Arc::new(Mutex::new(Vec::new()));
    let recorded = writes.clone();
    let writer = Writer::start(
        move |snapshot| {
            recorded.lock().unwrap().push(snapshot.active.clone());
            if failing.load(Ordering::SeqCst) {
                bail!("injected disk failure");
            }
            Ok(())
        },
        INTERVAL,
    )
    .unwrap();
    writer.submit(snapshot(1));
    assert!(
        writer
            .flush()
            .unwrap_err()
            .to_string()
            .contains("injected disk failure")
    );
    assert_eq!(
        writer
            .shared
            .0
            .lock()
            .unwrap()
            .pending
            .as_ref()
            .unwrap()
            .1
            .active,
        "1"
    );
    writer.submit(snapshot(2));
    fail.store(false, Ordering::SeqCst);
    writer.flush().unwrap();
    assert!(writer.error().is_none());
    fail.store(true, Ordering::SeqCst);
    writer.submit(snapshot(3));
    drop(writer); // Reports failure to stderr, joins and does not retry forever.
    assert_eq!(*writes.lock().unwrap(), ["1", "2", "3"]);
}

#[test]
fn panicking_write_is_reported_instead_of_stranding_flush() {
    let writer = Writer::start(|_| panic!("injected panic"), INTERVAL).unwrap();
    writer.submit(snapshot(1));
    assert!(
        writer
            .flush()
            .unwrap_err()
            .to_string()
            .contains("writer panicked")
    );
    drop(writer);
}

#[test]
fn main_pre_ui_save_preserves_legacy_until_atomic_migration_commits() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let legacy = config();
    fs::write(&path, toml::to_string(&legacy).unwrap()).unwrap();
    let mut startup = Config::load(&path).unwrap();
    startup.theme = "light".into();
    startup.active_tab = 0; // A save must preserve the *on-disk* legacy, not overwrite it.
    startup.save(&path).unwrap();
    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded.active_tab, 2);
    assert_eq!(loaded.tab_states["2"], legacy.tab_states["2"]);
    let (writer, imported, warning) = Writer::open(&path, &loaded, false).unwrap();
    assert!(warning.is_none());
    let imported = imported.unwrap();
    let mut restored = Config::load(&path).unwrap();
    assert!(restored.tab_states.is_empty());
    let selections = imported.restore(&mut restored);
    assert_eq!(restored.active_tab, 2);
    assert_eq!(restored.tab_states["2"], legacy.tab_states["2"]);
    assert_eq!(
        selections["2"],
        Selection::Item(legacy.route.clone().unwrap())
    );
    drop(writer);
    // Simulate a crash after committing SQLite but before removing old keys.
    fs::write(&path, toml::to_string(&legacy).unwrap()).unwrap();
    let mut conflicting = legacy.clone();
    conflicting.tab_states.get_mut("2").unwrap().field = 99;
    let (writer, repeated, _) = Writer::open(&path, &conflicting, false).unwrap();
    assert!(
        repeated.unwrap() == imported,
        "existing committed local state must win over legacy on retry"
    );
    drop(writer);
}

#[test]
fn legacy_open_routes_and_project_indices_infer_identity_without_an_item_index() {
    let mut legacy = config();
    legacy.selections.clear();
    let expected = Selection::Item(legacy.route.clone().unwrap());
    for tab_states in [
        legacy.tab_states.clone(),
        BTreeMap::new(),
        BTreeMap::from([("2".into(), TabState::default())]),
    ] {
        legacy.tab_states = tab_states;
        let snapshot = Snapshot::legacy(&legacy, false);
        let mut restored = legacy.clone();
        assert_eq!(snapshot.restore(&mut restored)["2"], expected);
        assert_eq!(restored.tab_states["2"].route, legacy.route);
    }
    legacy.section = Some(2);
    legacy.tab_states.insert("2".into(), TabState::default());
    let mut restored = legacy.clone();
    Snapshot::legacy(&legacy, false).restore(&mut restored);
    assert_eq!(restored.tab_states["2"].section_cursor, 2);
    legacy.active_tab = 1;
    legacy.route = None;
    legacy.tab_states.clear();
    legacy.project_route = Some(legacy.projects[2].id);
    let snapshot = Snapshot::legacy(&legacy, false);
    let mut restored = legacy.clone();
    assert_eq!(
        snapshot.restore(&mut restored)["1"],
        Selection::Project(legacy.projects[2].id)
    );
    legacy.project_route = None;
    legacy.selections.insert("1".into(), 3);
    let snapshot = Snapshot::legacy(&legacy, false);
    assert_eq!(
        snapshot.restore(&mut restored)["1"],
        Selection::Project(legacy.projects[3].id)
    );
    legacy.selections.insert("1".into(), 999);
    assert_eq!(
        Snapshot::legacy(&legacy, false).restore(&mut restored)["1"],
        Selection::Legacy(999)
    );
}

#[test]
fn legacy_selection_only_maps_and_global_route_are_imported_from_disk_not_constructor_defaults() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let mut legacy = config();
    legacy.tab_states.clear();
    legacy.section = Some(2);
    legacy.selections.insert("3".into(), 7);
    fs::write(&path, toml::to_string(&legacy).unwrap()).unwrap();
    let (writer, imported, _) = Writer::open(&path, &Config::default(), false).unwrap();
    let mut restored = legacy.clone();
    let selections = imported.unwrap().restore(&mut restored);
    assert_eq!(restored.active_tab, 2);
    assert_eq!(restored.tab_states["2"].route, legacy.route);
    assert_eq!(restored.tab_states["2"].section_cursor, 2);
    assert_eq!(restored.tab_states["3"].list_offset, 5);
    assert_eq!(selections["3"], Selection::Legacy(7));
    drop(writer);
}

#[cfg(unix)]
#[test]
fn safe_config_symlink_replacement_preserves_legacy_before_migration() {
    use std::os::unix::fs::symlink;
    let dir = tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let target = dir.path().join("old-config.toml");
    let legacy = config();
    let bytes = toml::to_string(&legacy).unwrap();
    fs::write(&target, &bytes).unwrap();
    symlink(&target, &path).unwrap();
    legacy.save(&path).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), bytes);
    assert_eq!(
        Config::load(&path).unwrap().tab_states["2"],
        legacy.tab_states["2"]
    );
    let (writer, snapshot, _) = Writer::open(&path, &Config::load(&path).unwrap(), false).unwrap();
    assert!(snapshot.is_some());
    assert!(Config::load(&path).unwrap().tab_states.is_empty());
    drop(writer);
}

#[test]
fn sqlite_open_and_migration_commit_failures_leave_recoverable_toml_untouched() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let config = config();
    fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    let dbpath = database_path(&path, &config.gitlab_url, false);
    fs::create_dir_all(dbpath.parent().unwrap()).unwrap();
    fs::write(&dbpath, "not sqlite").unwrap();
    assert!(Writer::open(&path, &config, false).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::remove_file(&dbpath).unwrap();
    let db = open_database(&dbpath).unwrap();
    db.execute_batch("CREATE TRIGGER fail_migration BEFORE INSERT ON snapshot BEGIN SELECT RAISE(ABORT, 'injected migration failure'); END;").unwrap();
    assert!(Writer::open(&path, &config, false).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(read_snapshot(&db).unwrap().is_none());
    db.execute_batch("DROP TRIGGER fail_migration").unwrap();
    let (writer, snapshot, warning) = Writer::open(&path, &config, false).unwrap();
    assert!(snapshot.is_some());
    assert!(warning.is_none());
    assert!(Config::load(&path).unwrap().tab_states.is_empty());
    drop(writer);
}

#[cfg(unix)]
#[test]
fn failed_workspace_directory_sync_keeps_legacy_even_after_sqlite_commit() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir().unwrap();
    let parent = dir.path().join("workspace");
    fs::create_dir(&parent).unwrap();
    let path = parent.join("config.toml");
    let config = config();
    fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    // Known names can still be read/created with write+execute, but opening the
    // directory for fsync requires read permission on this Unix test host.
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o300)).unwrap();
    if fs::File::open(&parent).is_ok() {
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        eprintln!("directory-read permissions are bypassed by this privileged test runner");
        return;
    }
    let result = Writer::open(&path, &config, false);
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("sync navigation directories")
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    let db = Connection::open(database_path(&path, &config.gitlab_url, false)).unwrap();
    assert!(
        read_snapshot(&db).unwrap().is_some(),
        "the local transaction already committed before the fsync guard failed"
    );
    let (writer, _, warning) = Writer::open(&path, &config, false).unwrap();
    assert!(warning.is_none());
    assert!(Config::load(&path).unwrap().tab_states.is_empty());
    drop(writer);
}

#[test]
fn failed_cleanup_reports_committed_migration_and_can_retry_without_data_loss() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let legacy = config();
    fs::write(&path, toml::to_string(&legacy).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    let mut invalid = legacy.clone();
    invalid.theme = "invalid".into(); // Inject a failure at the TOML cleanup boundary.
    let (writer, snapshot, warning) = Writer::open(&path, &invalid, false).unwrap();
    assert!(warning.unwrap().contains("cleanup failed"));
    assert_eq!(fs::read(&path).unwrap(), before);
    let snapshot = snapshot.unwrap();
    assert!(
        read_snapshot(&open_database(&database_path(&path, &legacy.gitlab_url, false)).unwrap())
            .unwrap()
            .unwrap()
            == snapshot
    );
    drop(writer);
    let (writer, retried, warning) = Writer::open(&path, &legacy, false).unwrap();
    assert!(retried.unwrap() == snapshot);
    assert!(warning.is_none());
    assert!(Config::load(&path).unwrap().tab_states.is_empty());
    drop(writer);
}

#[test]
fn view_identity_survives_reordering_but_not_replacement_or_external_rename() {
    let mut config = config();
    config.views = ["First", "Middle", "Last"]
        .into_iter()
        .map(|name| SavedView {
            name: name.into(),
            kind: ItemKind::Issue,
            query: "state:opened".into(),
        })
        .collect();
    config.active_tab = 6;
    config.tab_states.insert(
        "6".into(),
        TabState {
            field: 7,
            ..TabState::default()
        },
    );
    let selections = BTreeMap::from([("6".into(), Selection::Project(42))]);
    let snapshot = Snapshot::capture(&config, &selections, false);
    config.views.remove(1);
    config.views.swap(0, 1);
    let restored = snapshot.restore(&mut config);
    assert_eq!(config.active_tab, 4);
    assert_eq!(config.tab_states["4"].field, 7);
    assert_eq!(restored["4"], Selection::Project(42));
    config.views[0].name = "Replacement".into();
    snapshot.restore(&mut config);
    assert_eq!(config.active_tab, 0);
    assert!(!config.tab_states.contains_key("4"));
}

#[test]
fn filters_and_visibility_invalidate_stale_local_tab_state() {
    let config = config();
    let snapshot = Snapshot::legacy(&config, false);
    let mut filtered = config.clone();
    filtered.filters.insert("2".into(), "state:closed".into());
    assert!(snapshot.restore(&mut filtered).is_empty());
    assert!(filtered.tab_states.is_empty());
    let mut hidden = config.clone();
    hidden.projects[0].visible = false;
    assert!(snapshot.restore(&mut hidden).is_empty());
    let mut reordered = config.clone();
    reordered.projects.reverse();
    assert!(
        !snapshot.restore(&mut reordered).is_empty(),
        "project order does not invalidate identity"
    );
}

#[test]
fn token_rotation_and_content_cache_eviction_do_not_change_local_navigation() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let mut config = config();
    config.save(&path).unwrap();
    let (writer, _, _) = Writer::open(&path, &config, false).unwrap();
    let snapshot = Snapshot::legacy(&config, false);
    writer.submit(snapshot.clone());
    writer.flush().unwrap();
    drop(writer);
    let cache = ContentCache::open(&path, &config.gitlab_url, "old-token").unwrap();
    cache.clear().unwrap();
    drop(cache);
    let cache = ContentCache::open(&path, &config.gitlab_url, "new-token").unwrap();
    cache.clear().unwrap();
    drop(cache);
    config.token_env = "ROTATED_TOKEN".into();
    let (writer, restored, _) = Writer::open(&path, &config, false).unwrap();
    assert!(restored.unwrap() == snapshot);
    drop(writer);
    let (writer, demo_state, _) = Writer::open(&path, &config, true).unwrap();
    assert!(
        demo_state.is_none(),
        "demo and live must be separate even at the same explicit path"
    );
    drop(writer);
    config.gitlab_url = "https://another.example.com/gitlab".into();
    let (writer, other_host, _) = Writer::open(&path, &config, false).unwrap();
    assert!(other_host.is_none());
    drop(writer);
    let bytes = fs::read(database_path(&path, "https://gitlab.com", false)).unwrap();
    assert!(!bytes.windows(5).any(|w| w == b"token"));
}

#[test]
fn installation_changes_reuse_one_worker_and_copied_foreign_snapshots_are_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let mut config = config();
    config.save(&path).unwrap();
    let (writer, _, _) = Writer::open(&path, &config, false).unwrap();
    let original = Snapshot::legacy(&config, false);
    writer.submit(original.clone());
    writer.flush().unwrap();
    let thread_id = writer.thread.as_ref().unwrap().thread().id();
    config.gitlab_url = "https://other.example.com".into();
    let other = Snapshot::legacy(&config, false);
    writer.submit(other.clone());
    writer.flush().unwrap();
    assert_eq!(writer.thread.as_ref().unwrap().thread().id(), thread_id);
    drop(writer);
    let original_path = database_path(&path, "https://gitlab.com", false);
    let other_path = database_path(&path, &config.gitlab_url, false);
    assert!(
        read_snapshot(&Connection::open(&original_path).unwrap())
            .unwrap()
            .unwrap()
            == original
    );
    assert!(
        read_snapshot(&Connection::open(&other_path).unwrap())
            .unwrap()
            .unwrap()
            == other
    );
    fs::copy(original_path, other_path).unwrap();
    assert!(
        Writer::open(&path, &config, false).is_err(),
        "renaming a foreign database must not bypass installation isolation"
    );
}

#[cfg(unix)]
#[test]
fn private_files_and_unsafe_symlinks_are_checked_including_journal_sidecars() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let config = config();
    let dbpath = database_path(&path, &config.gitlab_url, false);
    let db = open_database(&dbpath).unwrap();
    assert_eq!(
        fs::metadata(&dbpath).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(dbpath.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    drop(db);
    let target = dir.path().join("do-not-touch");
    fs::write(&target, "private original").unwrap();
    fs::remove_file(&dbpath).unwrap();
    symlink(&target, &dbpath).unwrap();
    assert!(Writer::open(&path, &config, false).is_err());
    fs::remove_file(&dbpath).unwrap();
    let mut journal = dbpath.as_os_str().to_os_string();
    journal.push("-journal");
    symlink(&target, &journal).unwrap();
    assert!(Writer::open(&path, &config, false).is_err());
    fs::remove_file(journal).unwrap();
    fs::remove_dir(dbpath.parent().unwrap()).unwrap();
    symlink(dir.path(), dbpath.parent().unwrap()).unwrap();
    assert!(Writer::open(&path, &config, false).is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), "private original");
}

#[test]
fn informational_navigation_write_counts_and_persisted_bytes() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("workspace.toml");
    let config = config();
    config.save(&path).unwrap();
    let before = fs::read(&path).unwrap();
    let dbpath = database_path(&path, &config.gitlab_url, false);
    let mut db = open_database(&dbpath).unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let commits = count.clone();
    let writer = Writer::start(
        move |snapshot| {
            commit(&mut db, snapshot)?;
            commits.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
        INTERVAL,
    )
    .unwrap();
    let start = Instant::now();
    let mut schedule = Mailbox::default();
    for n in 0..10_000 {
        let now = start + Duration::from_millis(n as u64);
        let selections =
            BTreeMap::from([("2".into(), Selection::Item(config.route.clone().unwrap()))]);
        let mut navigation = Snapshot::capture(&config, &selections, false);
        navigation
            .tabs
            .get_mut("builtin:2")
            .unwrap()
            .state
            .content_offset = n;
        schedule.submit(navigation, now);
        if schedule.due(now, INTERVAL) {
            writer.submit(schedule.pending.take().unwrap().1);
            schedule.dirty_at = None;
            writer.flush().unwrap();
        }
    }
    writer.submit(schedule.pending.take().unwrap().1);
    writer.flush().unwrap();
    drop(writer);
    assert_eq!(count.load(Ordering::SeqCst), 4);
    assert_eq!(fs::read(&path).unwrap(), before);
    let db = Connection::open(&dbpath).unwrap();
    let body: String = db
        .query_row("SELECT body FROM snapshot", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Snapshot>(&body).unwrap().tabs["builtin:2"]
            .state
            .content_offset,
        9999
    );
    println!(
        "Informational fake-clock workload: 10,000 navigation updates / 10s; 0 TOML writes, 4 background SQLite commits; {} snapshot bytes, {} SQLite file bytes",
        body.len(),
        fs::metadata(&dbpath).unwrap().len()
    );
}
