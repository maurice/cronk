//! Machine-local navigation, deliberately independent of the credential content cache.
//! The UI only replaces a single pending snapshot. Serialization and SQLite commits
//! run on one owned worker; the first dirty deadline is never extended by input.
#[cfg(test)]
mod tests;
use crate::{
    config::{Config, NAVIGATION_KEYS, TabState},
    model::ItemKey,
};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const INTERVAL: Duration = Duration::from_millis(2500);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Selection {
    Item(ItemKey),
    Project(u64),
    // Only imported legacy TOML lacks identity. Resolved on first loaded list.
    Legacy(usize),
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Snapshot {
    namespace: String,
    active: String,
    tabs: BTreeMap<String, SavedTab>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SavedTab {
    context: String,
    state: TabState,
    selection: Option<Selection>,
}

fn hash(value: impl AsRef<[u8]>) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn identity(config: &Config, index: usize) -> String {
    if index < 4 {
        format!("builtin:{index}")
    } else {
        let view = &config.views[index - 4];
        format!(
            "view:{}",
            hash(
                serde_json::to_vec(&(view.name.as_str(), view.kind, view.query.as_str())).unwrap()
            )
        )
    }
}

fn context(config: &Config, index: usize) -> String {
    let query = config
        .filters
        .get(&index.to_string())
        .map(String::as_str)
        .unwrap_or_else(|| {
            config
                .views
                .get(index.wrapping_sub(4))
                .map_or("", |v| v.query.as_str())
        });
    // Conservative invalidation: changed project visibility or filter must not
    // restore an old detail/cursor into a now different list. Order is irrelevant.
    let mut projects: Vec<_> = config.projects.iter().map(|p| (p.id, p.visible)).collect();
    projects.sort_unstable();
    hash(serde_json::to_vec(&(query, projects)).unwrap())
}

impl Snapshot {
    pub(crate) fn capture(
        config: &Config,
        selections: &BTreeMap<String, Selection>,
        demo: bool,
    ) -> Self {
        let mut tabs = BTreeMap::new();
        for index in 0..4 + config.views.len() {
            let key = index.to_string();
            if let Some(state) = config.tab_states.get(&key) {
                tabs.insert(
                    identity(config, index),
                    SavedTab {
                        context: context(config, index),
                        state: state.clone(),
                        selection: selections.get(&key).cloned(),
                    },
                );
            }
        }
        Self {
            namespace: namespace(&config.gitlab_url, demo),
            active: identity(config, config.active_tab.min(3 + config.views.len())),
            tabs,
        }
    }

    fn legacy(config: &Config, demo: bool) -> Self {
        let mut config = config.clone();
        config
            .tab_states
            .entry(config.active_tab.to_string())
            .or_insert_with(|| TabState {
                route: config.route.clone(),
                project_route: config.project_route,
                section: config.section,
                field: config.field,
                section_cursor: config.section.unwrap_or(0),
                list_offset: config
                    .selections
                    .get(&config.active_tab.to_string())
                    .copied()
                    .unwrap_or(0)
                    .saturating_sub(2),
                ..TabState::default()
            });
        let active = config
            .tab_states
            .get_mut(&config.active_tab.to_string())
            .unwrap();
        if active.route.is_none() && active.project_route.is_none() {
            active.route = config.route.clone();
            active.project_route = config.project_route;
            if active.route.is_some() || active.project_route.is_some() {
                active.section = config.section;
                active.section_cursor = config.section.unwrap_or(active.section_cursor);
                active.field = config.field;
            }
        }
        // Older workspaces had per-tab selections but no complete TabState map.
        for (key, index) in &config.selections {
            config
                .tab_states
                .entry(key.clone())
                .or_insert_with(|| TabState {
                    list_offset: index.saturating_sub(2),
                    ..TabState::default()
                });
        }
        let selections = config
            .tab_states
            .iter()
            .filter_map(|(key, tab)| {
                // An open object is a stronger identity than the old sorted cursor,
                // including workspaces with no selections map at all.
                let project_route = (key == "1").then_some(tab.project_route).flatten();
                let selection = project_route
                    .map(Selection::Project)
                    .or_else(|| tab.route.clone().map(Selection::Item))
                    .or_else(|| tab.project_route.map(Selection::Project))
                    .or_else(|| {
                        config.selections.get(key).map(|index| {
                            if key == "1"
                                && let Some(project) = config.projects.get(*index)
                            {
                                Selection::Project(project.id)
                            } else {
                                Selection::Legacy(*index)
                            }
                        })
                    });
                selection.map(|selection| (key.clone(), selection))
            })
            .collect();
        Self::capture(&config, &selections, demo)
    }

    pub(crate) fn restore(&self, config: &mut Config) -> BTreeMap<String, Selection> {
        config.clear_navigation();
        let mut selections = BTreeMap::new();
        for index in 0..4 + config.views.len() {
            let id = identity(config, index);
            if id == self.active {
                config.active_tab = index;
            }
            if let Some(saved) = self
                .tabs
                .get(&id)
                .filter(|s| s.context == context(config, index))
            {
                let key = index.to_string();
                config.tab_states.insert(key.clone(), saved.state.clone());
                if let Some(selection) = &saved.selection {
                    if let Selection::Legacy(index) = selection {
                        config.selections.insert(key.clone(), *index);
                    }
                    selections.insert(key, selection.clone());
                }
            }
        }
        selections
    }
}

fn namespace(installation: &str, demo: bool) -> String {
    hash(format!("{}\0{demo}", installation.trim_end_matches('/')))
}

pub(crate) fn database_path(workspace: &Path, installation: &str, demo: bool) -> PathBuf {
    workspace
        .with_extension("state")
        .join(format!("{}.sqlite3", namespace(installation, demo)))
}

fn reject_symlink(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!(
            "Navigation state path must not be a symlink: {}",
            path.display()
        );
    }
    Ok(())
}

fn open_database(path: &Path) -> Result<Connection> {
    let directory = path.parent().unwrap();
    reject_symlink(directory)?;
    // As with the content cache, restrict the private directory and database.
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    reject_symlink(path)?;
    for suffix in ["-journal", "-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        reject_symlink(Path::new(&sidecar))?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    let mut db = Connection::open(path).context("open navigation database")?;
    db.busy_timeout(Duration::from_millis(250))?;
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version > 1 {
        bail!("Navigation database is newer than this version of Cronk");
    }
    db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")?;
    if version == 0 {
        let tx = db.transaction()?;
        tx.execute_batch("CREATE TABLE snapshot (id INTEGER PRIMARY KEY CHECK(id=1), body TEXT NOT NULL); PRAGMA user_version=1;")?;
        tx.commit()?;
    }
    Ok(db)
}

fn read_snapshot(db: &Connection) -> Result<Option<Snapshot>> {
    let body: Option<String> = db
        .query_row("SELECT body FROM snapshot WHERE id=1", [], |r| r.get(0))
        .optional()?;
    body.map(|body| Ok(serde_json::from_str(&body)?))
        .transpose()
}

fn commit(db: &mut Connection, snapshot: &Snapshot) -> Result<()> {
    let body = serde_json::to_string(snapshot)?;
    let tx = db.transaction()?;
    tx.execute("INSERT INTO snapshot(id,body) VALUES(1,?) ON CONFLICT(id) DO UPDATE SET body=excluded.body", [body])?;
    tx.commit()?;
    Ok(())
}

#[derive(Default)]
struct Mailbox {
    pending: Option<(u64, Snapshot)>,
    dirty_at: Option<Instant>,
    submitted: u64,
    completed: u64,
    force: bool,
    stopping: bool,
    error: Option<String>,
}

impl Mailbox {
    fn submit(&mut self, snapshot: Snapshot, now: Instant) {
        self.submitted += 1;
        self.pending = Some((self.submitted, snapshot));
        self.dirty_at.get_or_insert(now);
    }

    fn due(&self, now: Instant, interval: Duration) -> bool {
        self.pending.is_some()
            && (self.force
                || self.stopping
                || self
                    .dirty_at
                    .is_some_and(|t| now.saturating_duration_since(t) >= interval))
    }
}

type Shared = Arc<(Mutex<Mailbox>, Condvar)>;

pub(crate) struct Writer {
    shared: Shared,
    thread: Option<JoinHandle<()>>,
}

impl Writer {
    /// Startup is the sole synchronous migration boundary. Do not strip any
    /// legacy key until its complete snapshot has durably committed to SQLite.
    pub(crate) fn open(
        workspace: &Path,
        config: &Config,
        demo: bool,
    ) -> Result<(Self, Option<Snapshot>, Option<String>)> {
        let path = database_path(workspace, &config.gitlab_url, demo);
        let mut db = open_database(&path)?;
        let mut snapshot = read_snapshot(&db)?;
        let mut current_namespace = namespace(&config.gitlab_url, demo);
        if snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.namespace != current_namespace)
        {
            bail!("Navigation snapshot belongs to a different installation or mode");
        }
        let document: Option<toml::Table> = match fs::read_to_string(workspace) {
            Ok(source) => Some(toml::from_str(&source)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).context("read legacy navigation"),
        };
        let legacy = document
            .as_ref()
            .is_some_and(|d| NAVIGATION_KEYS.iter().any(|key| d.contains_key(*key)));
        let mut warning = None;
        if legacy {
            if snapshot.is_none() {
                let persisted: Config = document
                    .as_ref()
                    .unwrap()
                    .clone()
                    .try_into()
                    .context("read legacy TOML navigation")?;
                if persisted.gitlab_url.trim_end_matches('/')
                    != config.gitlab_url.trim_end_matches('/')
                {
                    bail!(
                        "Legacy navigation belongs to a different installation; use a separate workspace"
                    );
                }
                let imported = Snapshot::legacy(&persisted, demo);
                commit(&mut db, &imported).context("commit legacy navigation migration")?;
                snapshot = Some(imported);
            }
            #[cfg(unix)]
            {
                let directory = path.parent().unwrap();
                let parent = directory
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                // Both the database entry and a newly created .state directory
                // must be durable before replacing the recoverable legacy file.
                for directory in [directory, parent] {
                    fs::File::open(directory)
                        .and_then(|file| file.sync_all())
                        .context("sync navigation directories before legacy cleanup")?;
                }
            }
            // An interrupted cleanup retries without overwriting authoritative DB state.
            if let Err(error) = config.save_migrated(workspace) {
                warning = Some(format!(
                    "Navigation migrated, but legacy TOML cleanup failed: {error:#}"
                ));
            }
        }
        let directory = workspace.with_extension("state");
        let writer = Self::start(
            move |snapshot| {
                // Installation changes reuse this same serial worker. Opening the
                // new namespace and all subsequent commits stay off the UI thread.
                if snapshot.namespace != current_namespace {
                    if snapshot.namespace.len() != 64
                        || !snapshot.namespace.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        bail!("Invalid navigation namespace");
                    }
                    db = open_database(&directory.join(format!("{}.sqlite3", snapshot.namespace)))?;
                    current_namespace = snapshot.namespace.clone();
                }
                commit(&mut db, snapshot)
            },
            INTERVAL,
        )?;
        Ok((writer, snapshot, warning))
    }

    fn start(
        mut persist: impl FnMut(&Snapshot) -> Result<()> + Send + 'static,
        interval: Duration,
    ) -> Result<Self> {
        let shared = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let worker = shared.clone();
        let handle = thread::Builder::new()
            .name("cronk-navigation".into())
            .spawn(move || {
                let (mutex, wake) = &*worker;
                loop {
                    let mut state = mutex.lock().unwrap_or_else(|e| e.into_inner());
                    while !state.due(Instant::now(), interval) {
                        if state.stopping {
                            return;
                        }
                        let wait = state
                            .dirty_at
                            .map_or(interval, |t| interval.saturating_sub(t.elapsed()));
                        state = wake
                            .wait_timeout(state, wait)
                            .unwrap_or_else(|e| e.into_inner())
                            .0;
                    }
                    let (revision, snapshot) = state.pending.take().unwrap();
                    state.dirty_at = None;
                    state.force = false;
                    drop(state);
                    // Never hold the mailbox while serializing, locking SQLite, or fsyncing.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        persist(&snapshot)
                    }))
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("navigation writer panicked")));
                    let mut state = mutex.lock().unwrap_or_else(|e| e.into_inner());
                    state.completed = revision;
                    state.error = result
                        .err()
                        .map(|error| format!("Navigation not saved: {error:#}"));
                    if state.error.is_some() && state.pending.is_none() && !state.stopping {
                        state.pending = Some((revision, snapshot));
                        state.dirty_at = Some(Instant::now());
                    }
                    wake.notify_all();
                    if state.stopping && state.pending.is_none() {
                        return;
                    }
                }
            })
            .context("start navigation writer")?;
        Ok(Self {
            shared,
            thread: Some(handle),
        })
    }

    pub(crate) fn submit(&self, snapshot: Snapshot) {
        let (mutex, wake) = &*self.shared;
        mutex
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .submit(snapshot, Instant::now());
        wake.notify_all();
    }

    pub(crate) fn error(&self) -> Option<String> {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .error
            .clone()
    }

    pub(crate) fn flush(&self) -> Result<()> {
        let (mutex, wake) = &*self.shared;
        let mut state = mutex.lock().unwrap_or_else(|e| e.into_inner());
        // A failed revision retained for retry must be attempted again on flush.
        let target = state.submitted;
        if let Some((revision, _)) = state.pending.as_mut() {
            *revision = target + 1;
            state.submitted = target + 1;
        }
        let target = state.submitted;
        if state.pending.is_some() {
            state.force = true;
            wake.notify_all();
        }
        while state.completed < target {
            state = wake.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        if let Some(error) = &state.error {
            bail!("{error}");
        }
        Ok(())
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        {
            let (mutex, wake) = &*self.shared;
            let mut state = mutex.lock().unwrap_or_else(|e| e.into_inner());
            state.stopping = true;
            wake.notify_all();
        }
        if let Some(handle) = self.thread.take() {
            if handle.join().is_err() {
                eprintln!("Navigation writer failed during shutdown");
            } else if let Some(error) = self.error() {
                // Drop cannot propagate errors. Explicit orderly Quit uses flush
                // and keeps the UI open; abnormal teardown still reports failure.
                eprintln!("{error}");
            }
        }
    }
}
