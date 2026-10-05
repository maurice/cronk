mod completion;
mod controller;
mod interaction;
mod view;

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
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    time::Duration,
};
use tui_lipan::prelude::*;

pub struct Cronk {
    pub config: Config,
    pub path: Option<PathBuf>,
    pub api: Option<GitLab>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    List,
    Details,
    Section,
}

pub struct State {
    pub config: Config,
    feedback: interaction::Feedback,
    pub demo: bool,
    pub scope: Scope,
    pub items: Vec<WorkItem>,
    pub user: User,
    user_formatter: UserFormatter,
    pub details: Option<Details>,
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
    pub dialog: Option<Dialog>,
    pub current_iterations: HashMap<u64, Option<CurrentIteration>>,
    pub status: String,
    pub error: Option<String>,
    pub list_pending: HashSet<u64>,
    pub detail_pending: Option<(ItemKey, u64)>,
    pub trace_pending: HashSet<(u64, u64)>,
    pub mutation_pending: bool,
    pub user_pending: bool,
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
    next_details: Duration,
    next_traces: Duration,
}

#[derive(Default)]
pub struct Trace {
    pub text: String,
    pub offset: u64,
    pub finished: bool,
    pub error: Option<String>,
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
    Refresh,
    Filter,
    SaveView,
    RenameView,
    DeleteView,
    AddProject,
    RemoveProject,
    AliasProject,
    NewIssue,
    NewMergeRequest,
    Comment,
    Reply,
    Resolve,
    RetryJob,
    Themes,
    Help,
    Quit,
}

pub const COMMANDS: &[(&str, Action)] = &[
    (
        "Refresh now · all visible projects and current detail",
        Action::Refresh,
    ),
    ("Filter this view", Action::Filter),
    ("Save filter as a new tab", Action::SaveView),
    ("Rename saved tab", Action::RenameView),
    ("Delete saved tab", Action::DeleteView),
    ("Add existing GitLab project", Action::AddProject),
    ("Remove project from workspace", Action::RemoveProject),
    ("Rename project alias", Action::AliasProject),
    ("Create issue", Action::NewIssue),
    ("Create merge request", Action::NewMergeRequest),
    ("Add comment", Action::Comment),
    ("Reply to selected discussion", Action::Reply),
    ("Resolve / reopen selected discussion", Action::Resolve),
    ("Retry selected job", Action::RetryJob),
    ("Choose theme", Action::Themes),
    ("Keyboard help", Action::Help),
    ("Quit", Action::Quit),
];

pub enum Msg {
    Hover(String, bool),
    ClickFlash(String),
    EndClickFlash(String, u64),
    Tick,
    Refresh,
    LoadProject(u64, u64),
    LoadDetails,
    LoadTraces,
    LoadUser,
    UserLoaded(Result<User, String>),
    ProjectLoaded(
        u64,
        u64,
        Result<(Vec<WorkItem>, Option<CurrentIteration>), String>,
    ),
    DetailsLoaded(ItemKey, u64, Result<Box<Details>, String>),
    TraceLoaded(ItemKey, u64, u64, bool, Result<TraceChunk, String>),
    ProjectResolved(Result<Project, String>),
    MutationDone(Result<(), String>),
    Tab(usize),
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
    ToggleJob(u64),
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

impl State {
    pub fn tab_name(&self) -> &str {
        match self.config.active_tab {
            0 => "Dashboard",
            1 => "Projects",
            2 => "Issues",
            3 => "Merge Requests",
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
            .collect()
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
    pub fn render_user(&self, user: &User) -> String {
        self.user_formatter.render(user)
    }
    pub fn project(&self, id: u64) -> Option<&Project> {
        self.config.projects.iter().find(|p| p.id == id)
    }
    pub fn current_iteration(&self, project: u64) -> Option<&CurrentIteration> {
        self.current_iterations
            .get(&project)
            .and_then(Option::as_ref)
    }
    pub fn visible_items(&self) -> Vec<&WorkItem> {
        let query = Query::parse(self.query_text()).unwrap_or_default();
        let mut items: Vec<_> = self
            .items
            .iter()
            .filter(|item| {
                let Some(project) = self.project(item.key.project).filter(|p| p.visible) else {
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
            .collect();
        if self.config.active_tab == 0 {
            items.sort_by(|a, b| {
                a.attention_rank(self.user.id)
                    .cmp(&b.attention_rank(self.user.id))
                    .then(b.updated_at.cmp(&a.updated_at))
                    .then(a.key.iid.cmp(&b.key.iid))
            });
        } else {
            items.sort_by(|a, b| {
                b.updated_at
                    .cmp(&a.updated_at)
                    .then(a.key.project.cmp(&b.key.project))
                    .then(a.key.iid.cmp(&b.key.iid))
            });
        }
        items
    }
    pub fn sections(&self) -> &'static [&'static str] {
        if self
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
        let mut fields = vec![
            ("Title", "title", i.title.clone()),
            ("State", "state_event", i.state.clone()),
            (
                "Labels",
                "labels",
                i.labels
                    .iter()
                    .map(|l| l.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            (
                "Assignees",
                "assignee_ids",
                i.assignees
                    .iter()
                    .map(|u| format!("@{}", u.username))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            ("Milestone", "milestone_id", i.milestone.clone()),
        ];
        if i.key.kind == ItemKind::MergeRequest {
            fields.push(("Target branch", "target_branch", i.target_branch.clone()));
            fields.push((
                "Reviewers",
                "reviewer_ids",
                i.reviewers
                    .iter()
                    .map(|u| format!("@{}", u.username))
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        } else {
            fields.push(("Iteration", "iteration_id", i.iteration.clone()));
        }
        fields
    }
    pub fn command_options(&self) -> Vec<(&'static str, Action)> {
        let q = self
            .dialog
            .as_ref()
            .and_then(|d| d.fields.first())
            .map_or("", FormField::value)
            .to_lowercase();
        COMMANDS
            .iter()
            .copied()
            .filter(|(label, _)| label.to_lowercase().contains(&q))
            .collect()
    }
}

/// Dashboard rows have a fourth spacer row; other list rows occupy three rows.
pub fn list_row_height(active_tab: usize) -> usize {
    if active_tab == 0 { 4 } else { 3 }
}

/// Top and bottom chrome occupy eight rows; the remaining area determines visible items.
pub fn list_height_for_tab(viewport_height: u16, active_tab: usize) -> usize {
    (viewport_height.saturating_sub(8) as usize / list_row_height(active_tab)).max(1)
}

/// Visible list items for the standard three-row list layout.
pub fn list_height(viewport_height: u16) -> usize {
    list_height_for_tab(viewport_height, 1)
}
