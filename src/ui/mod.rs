mod completion;
mod controller;
mod interaction;
mod logs;
mod onboarding;
mod view;

pub use logs::{LogView, TraceIndex};

pub use completion::Completion;

use crate::{
    config::{Config, UserFormatter},
    filter::Query,
    gitlab::GitLab,
    model::*,
    scroll::BoundaryScroll,
};
use std::result::Result;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    time::Duration,
};
use tui_lipan::prelude::*;

/// Row of a section header key (`detail-section-N`), as opposed to its rows or gap.
fn is_section_header(key: &str) -> bool {
    key.strip_prefix("detail-section-")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Measure a detail target in content rows: `(top, end)` where `end` is exclusive
/// and includes a one-row gap. A section header spans to the next header (the
/// section's gap row is part of it). `end` is `None` when the extent lies beyond
/// the visible children, or when the target is not visible at all (`None` result).
pub(super) fn detail_span(
    event: &ScrollViewportEvent,
    target: &str,
) -> Option<(usize, Option<usize>)> {
    let position = event
        .visible
        .iter()
        .position(|child| child.key.as_ref().is_some_and(|key| key.as_ref() == target))?;
    let child = &event.visible[position];
    let top = child.content_rect.y.max(0) as usize;
    let bottom = |child: &ScrollVisibleChild| {
        (child.content_rect.y.max(0) as usize).saturating_add(child.content_rect.h as usize)
    };
    if !is_section_header(target) {
        if let Some(id) = target.strip_prefix("job-")
            && let Some(trace) = event.visible.get(position + 1).filter(|next| {
                next.key
                    .as_ref()
                    .is_some_and(|key| key.as_ref() == format!("trace-{id}"))
            })
        {
            return Some((top, Some(bottom(trace) + 1)));
        }
        return Some((top, Some(bottom(child) + 1)));
    }
    let next_header = event.visible[position + 1..].iter().find(|child| {
        child
            .key
            .as_ref()
            .is_some_and(|key| is_section_header(key.as_ref()))
    });
    let end = match next_header {
        Some(next) => Some(next.content_rect.y.max(0) as usize),
        None if event.last_visible_index == event.children_len.checked_sub(1) => {
            event.visible.last().map(bottom)
        }
        None => None,
    };
    Some((top, end))
}

pub struct Cronk {
    pub config: Config,
    pub path: Option<PathBuf>,
    pub api: Option<GitLab>,
    /// Explicit mode: first-run live setup may not have a client yet.
    pub demo: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    List,
    Details,
    Section,
}

pub struct State {
    pub config: Config,
    pub(super) saved_config: Config,
    pub(super) navigation_writer: Option<crate::navigation::Writer>,
    pub(super) navigation_snapshot: Option<crate::navigation::Snapshot>,
    pub(super) navigation_selections: BTreeMap<String, crate::navigation::Selection>,
    pub(super) navigation_restoring: HashSet<String>,
    pub(super) navigation_routes_restoring: HashSet<String>,
    pub(super) navigation_hydrated: HashSet<u64>,
    pub(super) navigation_error: Option<String>,
    pub(super) navigation_reported_error: Option<String>,
    pub(super) navigation_exit_failed: bool,
    feedback: interaction::Feedback,
    pub demo: bool,
    pub scope: Scope,
    pub items: Vec<WorkItem>,
    /// Placeholder rows for starred items that are not (yet) in `items`, so
    /// every star stays visible and removable on the Starred tab.
    pub(super) starred_stubs: Vec<WorkItem>,
    pub user: User,
    user_formatter: UserFormatter,
    pub details: Option<Details>,
    pub shared_details: HashMap<ItemKey, (Details, Duration)>,
    pub detail_requests: HashMap<ItemKey, u64>,
    /// Deterministic decode failures are retried only on explicit refresh.
    pub detail_schema_errors: HashMap<ItemKey, String>,
    pub detail_retry_requested: HashSet<ItemKey>,
    pub detail_sequence: u64,
    pub sync_progress: BTreeMap<u64, crate::gitlab::SyncProgress>,
    pub full_resync: bool,
    pub scroll: BoundaryScroll,
    pub section_cursor: usize,
    pub content_offset: usize,
    pub content_max_offset: usize,
    pub reveal_content: bool,
    pub(super) detail_viewport: Option<(ItemKey, ScrollViewportEvent)>,
    pub(super) detail_offset_request: Option<usize>,
    pub(super) tab_cache: BTreeMap<String, TabCache>,
    pub traces: HashMap<u64, Trace>,
    pub expanded: HashSet<u64>,
    pub collapsed: HashSet<u64>,
    pub log_views: HashMap<u64, LogView>,
    pub log_focus: Option<u64>,
    pub log_zoom: bool,
    pub log_zoom_from_focus: bool,
    pub log_search: Option<TextInput>,
    pub dialog: Option<Dialog>,
    pub onboarding: onboarding::SetupState,
    pub current_iterations: HashMap<u64, Option<CurrentIteration>>,
    pub status: String,
    pub error: Option<String>,
    pub list_pending: HashSet<u64>,
    pub detail_pending: Option<(ItemKey, u64)>,
    pub trace_pending: HashSet<(u64, u64)>,
    pub mutation_pending: bool,
    pub user_pending: bool,
    pub user_schema_error: bool,
    pub user_retry_requested: bool,
    pub user_epoch: u64,
    pub list_epoch: u64,
    pub detail_epoch: u64,
    pub next_lists: Duration,
    pub next_details: Duration,
    pub next_traces: Duration,
    pub failures: u32,
    pub blocked_until: Duration,
    pub tick: u64,
    pub lookup_epoch: u64,
    pub lookup_inflight: Option<u64>,
}

#[derive(Default)]
pub(super) struct TabCache {
    details: Option<Details>,
    traces: HashMap<u64, Trace>,
    log_views: HashMap<u64, LogView>,
    next_details: Duration,
    next_traces: Duration,
}

#[derive(Default)]
pub struct Trace {
    pub text: String,
    pub offset: u64,
    pub finished: bool,
    pub error: Option<String>,
    pub index: std::cell::RefCell<TraceIndex>,
}

pub struct FormField {
    pub label: String,
    pub input: TextInput,
    pub editor: TextEditor,
    pub multiline: bool,
    pub completion: Option<Completion>,
}
impl FormField {
    pub fn new(label: &str, value: &str, multiline: bool) -> Self {
        Self {
            label: label.into(),
            input: TextInput::new(value),
            editor: TextEditor::new(value),
            multiline,
            completion: None,
        }
    }
    pub fn uses_editor(&self) -> bool {
        self.multiline
            || self
                .completion
                .as_ref()
                .is_some_and(|c| c.kind == LookupKind::Labels)
    }
    pub fn value(&self) -> &str {
        if self.uses_editor() {
            self.editor.text()
        } else {
            self.input.text()
        }
    }
    pub fn cursor(&self) -> usize {
        if self.uses_editor() {
            self.editor.cursor()
        } else {
            self.input.cursor()
        }
    }
    pub fn set_text_and_cursor(&mut self, text: String, cursor: usize) {
        if self.uses_editor() {
            self.editor.set_text(&text);
            self.editor.set_cursor(cursor);
        }
        self.input = TextInput::new(text);
        self.input.set_cursor(cursor);
    }
}

pub struct Dialog {
    pub kind: DialogKind,
    pub title: String,
    pub help: String,
    pub fields: Vec<FormField>,
    pub original_theme: Option<String>,
    pub selected: usize,
    pub reveal_selection: bool,
    pub error: Option<String>,
}

#[derive(Clone)]
pub enum DialogKind {
    Onboarding,
    Commands,
    Themes,
    Filter,
    SaveView,
    RenameView,
    AddProject,
    AliasProject(u64),
    Edit(ItemKey, String),
    Comment(ItemKey),
    Reply(ItemKey, String),
    NewIssue,
    NewMergeRequest,
    Confirm(Confirmation),
    Help,
}
#[derive(Clone)]
pub enum Confirmation {
    RemoveProject(u64),
    DeleteView(usize),
    Mutate(Mutation),
}

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Onboarding,
    Refresh,
    FullResync,
    ClearCache,
    Filter,
    SaveView,
    RenameView,
    DeleteView,
    AddProject,
    RemoveProject,
    NewIssue,
    NewMergeRequest,
    Comment,
    Reply,
    Resolve,
    RetryJob,
    ToggleStar,
    Themes,
    Help,
    Quit,
}

pub const COMMANDS: &[(&str, Action)] = &[
    ("Set up GitLab · onboarding", Action::Onboarding),
    (
        "Refresh now · all visible projects and current detail",
        Action::Refresh,
    ),
    ("Full resync · reconcile all history", Action::FullResync),
    ("Clear local content cache", Action::ClearCache),
    ("Filter this view", Action::Filter),
    ("Save filter as a new tab", Action::SaveView),
    ("Rename saved tab", Action::RenameView),
    ("Delete saved tab", Action::DeleteView),
    ("Add project", Action::AddProject),
    ("Create issue", Action::NewIssue),
    ("Create merge request", Action::NewMergeRequest),
    ("Add comment", Action::Comment),
    ("Reply to selected discussion", Action::Reply),
    ("Resolve / reopen selected discussion", Action::Resolve),
    ("Retry selected job", Action::RetryJob),
    (
        "Star / unstar this issue or merge request",
        Action::ToggleStar,
    ),
    ("Choose theme", Action::Themes),
    ("Keyboard help", Action::Help),
    ("Quit", Action::Quit),
];

/// Extra search words for commands, so users can find them by other names.
fn command_aliases(action: Action) -> &'static [&'static str] {
    match action {
        Action::ToggleStar => &["bookmark", "favourite", "favorite", "starred"],
        _ => &[],
    }
}

pub enum Msg {
    Hover(String, bool),
    StatusHover(String, String, Option<(u16, u16)>),
    ClickFlash(String),
    EndClickFlash(String, u64),
    Tick,
    Refresh,
    FullResync,
    ClearCache,
    CacheWarning(ItemKey, u64, String),
    ProjectCached(u64, u64, Result<Vec<WorkItem>, String>),
    ProjectProgress(u64, u64, crate::gitlab::SyncProgress, Vec<WorkItem>),
    DetailCached(ItemKey, u64, Box<Details>),
    DetailProgress(ItemKey, u64, DetailPart, Box<Details>),
    DetailFinished(ItemKey, u64, Result<Box<Details>, String>),
    LoadProject(u64, u64),
    LoadDetails,
    LoadTraces,
    LoadUser,
    UserLoaded(u64, Result<User, String>),
    ProjectLoaded(
        u64,
        u64,
        Result<(Vec<WorkItem>, Option<CurrentIteration>), String>,
    ),
    DetailsLoaded(ItemKey, u64, Result<Box<Details>, String>),
    TraceLoaded(ItemKey, u64, u64, bool, Result<TraceChunk, String>),
    ProjectResolved(Result<Project, String>),
    SetupValidate(u64),
    SetupValidated(u64, Box<onboarding::Validation>),
    MutationDone(Result<(), String>),
    Tab(usize),
    /// Star or unstar a specific issue or merge request (row/header star icon).
    ToggleStar(ItemKey),
    /// Reorder the selected item on the Starred tab.
    MoveStar(isize),
    Move(isize),
    Select(usize),
    Activate(usize),
    Enter,
    Back,
    Section(usize),
    DetailSection(usize),
    DetailScrolled(usize),
    ScrollContent(isize),
    Field(usize),
    ContentScroll(usize),
    DetailViewport(
        Box<ScrollViewportEvent>,
        Option<String>,
        usize,
        ItemKey,
        u64,
    ),
    ListScroll(usize),
    ListViewportChanged,
    ToggleProject,
    ToggleProjectKind(ItemKind),
    ProjectDetailViewport(u64, u64, Box<ScrollViewportEvent>),
    ToggleJob(u64),
    FocusLog(u64),
    ZoomLog,
    ScrollLog(u64, isize),
    LogWheel(u64, usize, isize),
    LogBar(u64, usize, usize, bool),
    LogBarEnd(u64),
    SearchLog,
    LogSearchInput(InputEvent),
    SubmitLogSearch,
    LogSearchNext(bool),
    Action(Action),
    Palette,
    CloseDialog,
    Submit,
    DialogScrolled,
    DialogSelect(usize),
    DialogField(usize),
    Input(usize, InputEvent),
    Editor(usize, TextAreaEvent),
    Newline(usize),

    LookupStart(u64, usize),
    LookupLoaded(u64, usize, Result<Vec<LookupOption>, String>),
    LookupMove(usize, isize),
    LookupAccept(usize, u64, usize),
    LookupEnter(usize),
}

/// The same definitions drive editing and the shared issue/MR label column.
fn editable_field_definitions(
    kind: ItemKind,
) -> impl Iterator<Item = (&'static str, &'static str)> {
    let common = [
        ("Title", "title"),
        ("State", "state_event"),
        ("Labels", "labels"),
        ("Assignees", "assignee_ids"),
        ("Milestone", "milestone_id"),
    ];
    let specific = match kind {
        ItemKind::Issue => [("Iteration", "iteration_id"), ("Parent (epic)", "epic_id")],
        ItemKind::MergeRequest => [
            ("Target branch", "target_branch"),
            ("Reviewers", "reviewer_ids"),
        ],
    };
    common.into_iter().chain(specific)
}

impl State {
    /// Discard client metadata. Demo has no network hydration, so reconstruct
    /// its current iterations only from the freshly installed fictional items.
    pub(super) fn reset_current_iterations(&mut self) {
        self.current_iterations.clear();
        if self.demo {
            for item in &self.items {
                if let (Some(id), false) = (item.iteration_id, item.iteration.trim().is_empty()) {
                    self.current_iterations
                        .entry(item.key.project)
                        .or_insert_with(|| {
                            Some(CurrentIteration {
                                id,
                                title: item.iteration.clone(),
                                description: "Current iteration · demo".into(),
                            })
                        });
                }
            }
        }
    }

    /// Explicit durability boundary for orderly exit and headless callers.
    /// Routine UI commits only enqueue a bounded latest snapshot.
    pub fn flush_navigation(&self) -> anyhow::Result<()> {
        if let Some(writer) = &self.navigation_writer {
            writer.flush()?;
        } else if let Some(error) = &self.navigation_error {
            anyhow::bail!("{error}");
        }
        Ok(())
    }

    pub(super) fn selection_index(
        &self,
        selection: &crate::navigation::Selection,
    ) -> Option<usize> {
        use crate::navigation::Selection;
        match selection {
            Selection::Item(selected) => self.visible_position(selected),
            Selection::Project(id) => self.config.projects.iter().position(|p| &p.id == id),
            Selection::Legacy(index) => Some(*index),
        }
    }

    pub(super) fn remember_selection(&mut self) {
        use crate::navigation::Selection;
        let key = self.tab_key();
        let open = (self.scope != Scope::List)
            .then(|| {
                if self.config.active_tab == 1 {
                    self.config.project_route.map(Selection::Project)
                } else {
                    self.config.route.clone().map(Selection::Item)
                }
            })
            .flatten()
            .filter(|selection| match selection {
                Selection::Item(item) => self
                    .config
                    .projects
                    .iter()
                    .any(|p| p.id == item.project && p.kind_visible(item.kind)),
                Selection::Project(id) => self.config.projects.iter().any(|p| p.id == *id),
                Selection::Legacy(_) => false,
            });
        if let Some(open) = open {
            // Detail refreshes can reorder the list in place. Its old numeric
            // slot must never replace the identity of the object being read.
            if let Some(index) = self.selection_index(&open) {
                self.scroll.selected = index;
                self.config.selections.insert(key.clone(), index);
            } else {
                self.navigation_restoring.insert(key.clone());
            }
            self.navigation_selections.insert(key, open);
            return;
        }
        if self.navigation_restoring.contains(&key) {
            return;
        }
        let selected = if self.config.active_tab == 1 {
            self.config
                .projects
                .get(self.scroll.selected)
                .map(|p| Selection::Project(p.id))
        } else {
            self.visible_item_at(self.scroll.selected)
                .map(|i| Selection::Item(i.key.clone()))
        };
        if let Some(selected) = selected {
            self.navigation_selections.insert(key, selected);
        } else {
            self.navigation_selections.remove(&key);
        }
    }

    pub(super) fn invalidate_hidden_navigation(&mut self) {
        use crate::navigation::Selection;
        let invalid: Vec<_> = self
            .navigation_selections
            .iter()
            .filter_map(|(key, selection)| {
                let valid = match selection {
                    Selection::Item(item) => self
                        .config
                        .projects
                        .iter()
                        .any(|p| p.id == item.project && p.kind_visible(item.kind)),
                    Selection::Project(id) => self.config.projects.iter().any(|p| p.id == *id),
                    Selection::Legacy(_) => true,
                };
                (!valid).then(|| key.clone())
            })
            .collect();
        for key in invalid {
            self.navigation_selections.remove(&key);
            self.navigation_restoring.remove(&key);
            self.navigation_routes_restoring.remove(&key);
            self.config.selections.insert(key.clone(), 0);
            if let Some(tab) = self.config.tab_states.get_mut(&key) {
                tab.list_offset = 0;
            }
            if key == self.tab_key() {
                self.scroll = BoundaryScroll::default();
            }
        }
        self.navigation_routes_restoring.retain(|key| {
            self.config
                .tab_states
                .get(key)
                .is_some_and(|tab| tab.route.is_some() || tab.project_route.is_some())
        });
    }

    pub(super) fn navigation_hydration_complete(&self) -> bool {
        self.config
            .projects
            .iter()
            .filter(|p| p.effectively_visible())
            .all(|p| self.navigation_hydrated.contains(&p.id))
    }

    pub(super) fn restore_selection(&mut self, complete: bool) {
        use crate::navigation::Selection;
        let key = self.tab_key();
        let restore_selection = self.navigation_restoring.contains(&key);
        let restore_route = self.navigation_routes_restoring.contains(&key);
        if !restore_selection && !restore_route {
            return;
        }
        // @me/dashboard predicates require the authenticated identity.
        if !self.demo && self.config.active_tab != 1 && self.user.id == 0 {
            return;
        }
        let selection = self.navigation_selections.get(&key);
        let selection_complete = complete
            || match selection {
                Some(Selection::Item(item)) => self.navigation_hydrated.contains(&item.project),
                Some(Selection::Project(_)) => true, // Portable project list is authoritative.
                Some(Selection::Legacy(_)) | None => false,
            };
        let index = selection.and_then(|selection| self.selection_index(selection));
        if restore_selection {
            if let Some(index) = index {
                self.scroll.selected = index;
            } else if selection_complete {
                self.scroll.selected = 0;
                self.scroll.offset = 0;
            }
        }
        let route_complete = complete
            || self
                .config
                .route
                .as_ref()
                .is_some_and(|route| self.navigation_hydrated.contains(&route.project))
            || self.config.project_route.is_some();
        // Validate each known identity against its owning project's successful
        // snapshot. An unrelated failure must neither delete it nor retain it
        // forever. Only anonymous legacy indices need global hydration proof.
        if route_complete
            && restore_route
            && !self.project_details()
            && self
                .config
                .route
                .as_ref()
                .is_some_and(|route| self.visible_position(route).is_none())
        {
            self.config.route = None;
            self.config.section = None;
            self.config.field = 0;
            self.scope = Scope::List;
            self.details = None;
            self.content_offset = 0;
            self.expanded.clear();
            self.collapsed.clear();
        }
        if selection_complete {
            self.navigation_restoring.remove(&key);
        }
        if route_complete {
            self.navigation_routes_restoring.remove(&key);
        }
    }

    pub fn selected_job(&self) -> Option<&Job> {
        (self.scope == Scope::Section && self.section_name() == "Jobs")
            .then(|| self.details.as_ref()?.jobs.get(self.config.field))
            .flatten()
    }

    pub fn job_expanded(&self, job: &Job) -> bool {
        !self.collapsed.contains(&job.id)
            && (job.running() || job.failed_hard() || self.expanded.contains(&job.id))
    }

    /// Whether this job's trace should currently be fetched or polled.
    pub fn trace_wanted(&self, job: &Job) -> bool {
        let loaded = self.traces.get(&job.id);
        (job.running()
            || (job.failed_hard() && !self.collapsed.contains(&job.id))
            || self.expanded.contains(&job.id)
            || loaded.is_some())
            && !loaded.is_some_and(|t| t.finished && !job.running())
    }

    pub fn log_height(&self, viewport_height: u16, id: u64) -> usize {
        if self.log_zoom && self.log_focus == Some(id) {
            let extra = usize::from(self.traces.get(&id).is_some_and(|t| t.error.is_some()))
                + usize::from(self.log_views.get(&id).is_some_and(|v| !v.query.is_empty()));
            (viewport_height as usize)
                .saturating_sub(2 + extra + usize::from(self.log_search.is_some()))
                .max(1)
        } else {
            self.details
                .as_ref()
                .and_then(|d| d.jobs.iter().find(|j| j.id == id))
                .map_or(8, |j| if j.running() { 8 } else { 18 })
        }
    }

    pub fn tab_name(&self) -> &str {
        match self.config.active_tab {
            0 => "Dashboard",
            1 => "Projects",
            2 => "Issues",
            3 => "Merge Requests",
            n if self.config.starred_tab() == Some(n) => "Starred",
            n => self
                .config
                .views
                .get(n - 4)
                .map_or("Dashboard", |v| v.name.as_str()),
        }
    }
    pub fn tab_names(&self) -> Vec<String> {
        ["Dashboard", "Projects", "Issues", "Merge Requests"]
            .into_iter()
            .map(str::to_owned)
            .chain(self.config.views.iter().map(|v| v.name.clone()))
            .chain(self.config.starred_tab().map(|_| "Starred".to_owned()))
            .collect()
    }
    /// The Starred tab lists every starred item in its saved, user-chosen order.
    pub fn starred_active(&self) -> bool {
        // Cheap checks first: this runs on every tab, for every list build.
        self.config.active_tab == 4 + self.config.views.len()
            && !self.config.starred.is_empty()
            && self.config.starred_tab() == Some(self.config.active_tab)
    }

    /// Display rank of every starred item, when the Starred tab is active.
    fn star_ranks(&self) -> Option<HashMap<ItemKey, usize>> {
        self.starred_active().then(|| {
            self.config
                .starred_keys()
                .into_iter()
                .enumerate()
                .map(|(rank, key)| (key, rank))
                .collect()
        })
    }

    /// Rebuild placeholder rows for starred items missing from `items`.
    pub(super) fn refresh_starred_stubs(&mut self) {
        if self.config.starred.is_empty() {
            self.starred_stubs.clear();
            return;
        }
        let keys = self.config.starred_keys();
        let loaded: HashSet<&ItemKey> = self.items.iter().map(|i| &i.key).collect();
        let stubs: Vec<WorkItem> = keys
            .into_iter()
            .filter(|key| !loaded.contains(key))
            .map(|key| WorkItem {
                title: self.config.starred_title(&key).unwrap_or_default().into(),
                state: "opened".into(),
                key,
                ..WorkItem::default()
            })
            .collect();
        self.starred_stubs = stubs;
    }

    /// Item the star toggle applies to: the open item, else the selected row.
    pub fn star_target(&self) -> Option<(ItemKey, String)> {
        if self.scope != Scope::List {
            let key = self.config.route.as_ref()?;
            let title = self
                .details
                .as_ref()
                .filter(|d| d.item.key == *key)
                .map(|d| d.item.title.clone())
                .or_else(|| {
                    self.items
                        .iter()
                        .find(|i| i.key == *key)
                        .map(|i| i.title.clone())
                })
                .unwrap_or_default();
            return Some((key.clone(), title));
        }
        if self.config.active_tab == 1 {
            return None;
        }
        self.visible_item_at(self.scroll.selected)
            .map(|i| (i.key.clone(), i.title.clone()))
    }
    pub fn tab_key(&self) -> String {
        self.config.active_tab.to_string()
    }
    pub fn query_text(&self) -> &str {
        self.config
            .filters
            .get(&self.tab_key())
            .map(String::as_str)
            .unwrap_or_else(|| {
                self.config
                    .active_tab
                    .checked_sub(4)
                    .and_then(|i| self.config.views.get(i))
                    .map_or("", |v| v.query.as_str())
            })
    }
    pub fn kind(&self) -> Option<ItemKind> {
        match self.config.active_tab {
            2 => Some(ItemKind::Issue),
            3 => Some(ItemKind::MergeRequest),
            n if n >= 4 => self.config.views.get(n - 4).map(|v| v.kind),
            _ => None,
        }
    }
    pub fn is_current_user(&self, id: u64) -> bool {
        self.user.id != 0 && self.user.id == id
    }
    pub fn render_user(&self, user: &User) -> String {
        if self.is_current_user(user.id) {
            "You".into()
        } else {
            self.user_formatter.render(user)
        }
    }
    pub fn render_user_lookup(&self, option: &LookupOption) -> String {
        if self.is_current_user(option.id) {
            "You".into()
        } else {
            self.user_formatter.render_lookup(option)
        }
    }
    pub fn project(&self, id: u64) -> Option<&Project> {
        self.config.projects.iter().find(|p| p.id == id)
    }
    pub fn current_iteration(&self, project: u64) -> Option<&CurrentIteration> {
        self.current_iterations
            .get(&project)
            .and_then(Option::as_ref)
    }
    pub fn visible_item_count(&self) -> usize {
        let ranks = self.star_ranks();
        self.matching_items_ranked(ranks.as_ref()).count()
    }

    fn matching_items_ranked<'a, 'b>(
        &'a self,
        ranks: Option<&'b HashMap<ItemKey, usize>>,
    ) -> impl Iterator<Item = &'a WorkItem> + 'b
    where
        'a: 'b,
    {
        let query = Query::parse(self.query_text()).unwrap_or_default();
        // Stubs stand in only for starred items that are not loaded.
        let loaded: HashSet<&ItemKey> = if ranks.is_some() && !self.starred_stubs.is_empty() {
            self.items.iter().map(|i| &i.key).collect()
        } else {
            HashSet::new()
        };
        let stubs = if ranks.is_some() {
            &self.starred_stubs[..]
        } else {
            &[]
        };
        let stubs = stubs.iter().filter(move |item| !loaded.contains(&item.key));
        self.items.iter().chain(stubs).filter(move |item| {
            if ranks.is_some_and(|r| !r.contains_key(&item.key)) {
                return false;
            }
            let Some(project) = self
                .project(item.key.project)
                .filter(|p| p.kind_visible(item.key.kind))
            else {
                return false;
            };
            if let Some(kind) = self.kind()
                && item.key.kind != kind
            {
                return false;
            }
            if self.config.active_tab == 0
                && (item.state != "opened" || !item.on_dashboard_for(self.user.id))
            {
                return false;
            }
            query.matches(
                item,
                project,
                &self.user.username,
                self.current_iteration(item.key.project),
            )
        })
    }

    /// Total order of the list: dashboard urgency, or newest first elsewhere.
    fn visible_order(
        &self,
        ranks: Option<&HashMap<ItemKey, usize>>,
        a: &WorkItem,
        b: &WorkItem,
    ) -> Ordering {
        if let Some(ranks) = ranks {
            // Saved order, which the user controls with Shift+Up / Shift+Down.
            ranks.get(&a.key).cmp(&ranks.get(&b.key))
        } else if self.config.active_tab == 0 {
            a.attention_rank(self.user.id)
                .cmp(&b.attention_rank(self.user.id))
                .then(b.updated_at.cmp(&a.updated_at))
                .then(a.key.iid.cmp(&b.key.iid))
        } else {
            b.updated_at
                .cmp(&a.updated_at)
                .then(a.key.project.cmp(&b.key.project))
                .then(a.key.iid.cmp(&b.key.iid))
        }
    }

    pub fn visible_items(&self) -> Vec<&WorkItem> {
        let ranks = self.star_ranks();
        let mut items: Vec<_> = self.matching_items_ranked(ranks.as_ref()).collect();
        items.sort_by(|a, b| self.visible_order(ranks.as_ref(), a, b));
        items
    }

    /// `visible_items()[index]` with one filter pass and no sort. Ties in
    /// `visible_order` fall back to filter order, as the stable sort does.
    pub fn visible_item_at(&self, index: usize) -> Option<&WorkItem> {
        let ranks = self.star_ranks();
        let mut items: Vec<_> = self
            .matching_items_ranked(ranks.as_ref())
            .enumerate()
            .collect();
        if index >= items.len() {
            return None;
        }
        let (_, (_, item), _) = items.select_nth_unstable_by(index, |(a_at, a), (b_at, b)| {
            self.visible_order(ranks.as_ref(), a, b)
                .then(a_at.cmp(b_at))
        });
        Some(item)
    }

    /// `visible_items().position(|item| item.key == *key)` with one filter
    /// pass and no sort: the rank of the first matching item is the number of
    /// items that sort before it.
    pub fn visible_position(&self, key: &ItemKey) -> Option<usize> {
        let ranks = self.star_ranks();
        let items: Vec<_> = self
            .matching_items_ranked(ranks.as_ref())
            .enumerate()
            .collect();
        let order = |(a_at, a): &(usize, &WorkItem), (b_at, b): &(usize, &WorkItem)| {
            self.visible_order(ranks.as_ref(), a, b)
                .then(a_at.cmp(b_at))
        };
        let first = items
            .iter()
            .filter(|(_, item)| item.key == *key)
            .min_by(|a, b| order(a, b))?;
        Some(
            items
                .iter()
                .filter(|other| order(other, first).is_lt())
                .count(),
        )
    }
    pub fn project_details(&self) -> bool {
        self.config.active_tab == 1 && self.config.project_route.is_some()
    }

    pub fn sections(&self) -> &'static [&'static str] {
        if self.project_details() {
            &["Fields", "Pipelines", "Forget this project"]
        } else if self
            .config
            .route
            .as_ref()
            .is_some_and(|k| k.kind == ItemKind::MergeRequest)
        {
            &[
                "Fields",
                "Description",
                "Pipeline",
                "Jobs",
                "Discussions",
                "Changes",
            ]
        } else {
            &["Fields", "Description", "Activity"]
        }
    }
    pub fn section_name(&self) -> &str {
        self.sections()
            .get(self.section_cursor)
            .copied()
            .unwrap_or("Fields")
    }
    pub(super) fn detail_target_key(&self) -> String {
        if self.scope == Scope::Section {
            match self.section_name() {
                "Forget this project" => return "project-remove".into(),
                "Fields" => return format!("edit-field-{}", self.config.field),
                "Jobs" => {
                    if let Some(job) = self
                        .details
                        .as_ref()
                        .and_then(|d| d.jobs.get(self.config.field))
                    {
                        return format!("job-{}", job.id);
                    }
                }
                "Discussions" => {
                    if let Some(discussion) = self
                        .details
                        .as_ref()
                        .and_then(|d| d.discussions.get(self.config.field))
                    {
                        return format!("discussion-{}", discussion.id);
                    }
                }
                _ => {}
            }
        }
        format!("detail-section-{}", self.section_cursor)
    }

    pub fn fields(&self) -> Vec<(&'static str, &'static str, String)> {
        let Some(d) = &self.details else {
            return vec![];
        };
        let i = &d.item;
        editable_field_definitions(i.key.kind)
            .map(|(label, name)| {
                let value = match name {
                    "title" => i.title.clone(),
                    "state_event" => i.state.clone(),
                    "labels" => i
                        .labels
                        .iter()
                        .map(|l| l.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    "assignee_ids" | "reviewer_ids" => {
                        let users = if name == "assignee_ids" {
                            &i.assignees
                        } else {
                            &i.reviewers
                        };
                        users
                            .iter()
                            .map(|u| format!("@{}", u.username))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                    "milestone_id" => i.milestone.clone(),
                    "iteration_id" => i.iteration.clone(),
                    "epic_id" => i.epic.clone(),
                    "target_branch" => i.target_branch.clone(),
                    _ => unreachable!("unknown editable field"),
                };
                (label, name, value)
            })
            .collect()
    }
    pub fn command_options(&self) -> Vec<(&'static str, Action)> {
        let q = self
            .dialog
            .as_ref()
            .and_then(|d| d.fields.first())
            .map_or("", FormField::value);
        let mut matches: Vec<_> = COMMANDS
            .iter()
            .copied()
            .filter_map(|c| {
                command_match(c.0, q)
                    .or_else(|| {
                        command_aliases(c.1)
                            .iter()
                            .any(|alias| command_match(alias, q).is_some())
                            .then_some(true)
                    })
                    .map(|in_order| (!in_order, c))
            })
            .collect();
        // Stable: label-order matches first, otherwise declaration order.
        matches.sort_by_key(|(out_of_order, _)| *out_of_order);
        matches.into_iter().map(|(_, c)| c).collect()
    }
}

/// Order-independent word-prefix match: every whitespace-separated query token
/// must be a prefix of a distinct word in `label` (case-insensitive). Returns
/// `Some(true)` when the tokens also match words in label order.
fn command_match(label: &str, query: &str) -> Option<bool> {
    let label = label.to_lowercase();
    let words: Vec<&str> = label
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let query = query.to_lowercase();
    let mut used = vec![false; words.len()];
    let (mut in_order, mut last) = (true, 0);
    for token in query.split_whitespace() {
        let i = (0..words.len()).find(|&i| !used[i] && words[i].starts_with(token))?;
        used[i] = true;
        in_order &= i >= last;
        last = i;
    }
    Some(in_order)
}

/// Dashboard and Projects rows include a fourth spacer row; work-list rows occupy three.
pub fn list_row_height(active_tab: usize) -> usize {
    if matches!(active_tab, 0 | 1) { 4 } else { 3 }
}

/// Top and bottom chrome occupy eight rows; the remaining area determines visible items.
pub fn list_height_for_tab(viewport_height: u16, active_tab: usize) -> usize {
    (viewport_height.saturating_sub(8) as usize / list_row_height(active_tab)).max(1)
}

/// Visible list items for the standard three-row work-list layout.
pub fn list_height(viewport_height: u16) -> usize {
    list_height_for_tab(viewport_height, 2)
}

#[cfg(test)]
mod command_match_tests {
    use super::command_match;

    #[test]
    fn matches_word_prefixes_in_any_order() {
        for q in ["foo b", "foo baz", "baz f", "baz foo", "FOO", "", "  b  f "] {
            assert!(command_match("foo bar baz", q).is_some(), "{q:?}");
        }
        for q in ["foo foo", "bar bar", "oo", "foo x", "foobar"] {
            assert!(command_match("foo bar baz", q).is_none(), "{q:?}");
        }
    }

    #[test]
    fn reports_label_order_and_splits_on_punctuation() {
        assert_eq!(command_match("foo bar baz", "foo b"), Some(true));
        assert_eq!(command_match("foo bar baz", "baz f"), Some(false));
        assert!(command_match("Resolve / reopen · x", "reo res").is_some());
    }
}
