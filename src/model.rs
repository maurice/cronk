use serde::{Deserialize, Serialize};
use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    #[default]
    Issue,
    MergeRequest,
}

impl ItemKind {
    pub fn segment(self) -> &'static str {
        match self {
            Self::Issue => "issues",
            Self::MergeRequest => "merge_requests",
        }
    }
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Issue => "#",
            Self::MergeRequest => "!",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub id: u64,
    pub path: String,
    /// Local display override. Empty falls back to the GitLab path.
    /// New projects may start with GitLab's short name when the add dialog leaves this blank.
    pub alias: String,
    pub visible: bool,
}

impl Project {
    pub fn name(&self) -> &str {
        if self.alias.is_empty() {
            &self.path
        } else {
            &self.alias
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ItemKey {
    pub project: u64,
    pub iid: u64,
    pub kind: ItemKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Label {
    pub name: String,
    pub color: String,
    pub text_color: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct User {
    pub id: u64,
    pub username: String,
    pub name: String,
    /// GitLab exposes this only when the user has chosen to make it public.
    pub public_email: Option<String>,
    pub bot: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LookupKind {
    Users,
    Milestones,
    Iterations,
    Epics,
    Projects,
    Labels,
}

pub const CURRENT_ITERATION_NAME: &str = "Current";

pub fn is_current_iteration_value(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case(CURRENT_ITERATION_NAME)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CurrentIteration {
    pub id: u64,
    pub title: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookupOption {
    pub id: u64,
    pub label: String,
    pub value: String,
    pub api_value: String,
    pub description: String,
    pub color: String,
    pub text_color: String,
}

impl LookupOption {
    pub fn user(user: &User) -> Self {
        Self {
            id: user.id,
            label: user.name.clone(),
            value: format!("@{}", user.username),
            api_value: user.id.to_string(),
            description: format!("@{} · ID {}", user.username, user.id),
            color: String::new(),
            text_color: String::new(),
        }
    }
    pub fn project(project: &Project) -> Self {
        Self {
            id: project.id,
            label: project.name().to_owned(),
            value: project.path.clone(),
            api_value: project.id.to_string(),
            description: format!("{} · ID {}", project.path, project.id),
            color: String::new(),
            text_color: String::new(),
        }
    }
    pub fn label(label: &Label) -> Self {
        Self {
            id: lookup_id(&label.name),
            label: label.name.clone(),
            value: label.name.clone(),
            api_value: label.name.clone(),
            description: "Known label".into(),
            color: label.color.clone(),
            text_color: label.text_color.clone(),
        }
    }

    pub fn current_iteration(iteration: &CurrentIteration) -> Self {
        Self {
            id: iteration.id,
            label: CURRENT_ITERATION_NAME.into(),
            value: CURRENT_ITERATION_NAME.into(),
            api_value: iteration.id.to_string(),
            description: format!("Resolves to {}", iteration.description),
            color: String::new(),
            text_color: String::new(),
        }
    }
}

fn lookup_id(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish().max(1)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pipeline {
    pub id: u64,
    pub status: String,
    pub web_url: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkItem {
    pub key: ItemKey,
    pub title: String,
    pub description: String,
    pub state: String,
    pub labels: Vec<Label>,
    pub author: User,
    pub assignees: Vec<User>,
    pub reviewers: Vec<User>,
    pub milestone: String,
    pub milestone_id: Option<u64>,
    pub iteration: String,
    pub iteration_id: Option<u64>,
    /// GitLab REST issue parent (epic); absent on installations without epics.
    pub epic: String,
    pub epic_id: Option<u64>,
    pub source_branch: String,
    pub target_branch: String,
    pub updated_at: String,
    pub web_url: String,
    pub draft: bool,
    pub pipeline: Option<Pipeline>,
    // None means GitLab has not supplied this information, not zero discussions.
    pub unresolved: Option<usize>,
}

impl WorkItem {
    pub fn dashboard_roles(&self, current_user: u64) -> Vec<&'static str> {
        if current_user == 0 {
            return Vec::new();
        }
        let mut roles = Vec::new();
        if self.author.id == current_user {
            roles.push("author");
        }
        if self.assignees.iter().any(|user| user.id == current_user) {
            roles.push("assignee");
        }
        if self.key.kind == ItemKind::MergeRequest
            && self.reviewers.iter().any(|user| user.id == current_user)
        {
            roles.push("reviewer");
        }
        roles
    }

    pub fn on_dashboard_for(&self, current_user: u64) -> bool {
        current_user != 0
            && (self.author.id == current_user
                || self.assignees.iter().any(|user| user.id == current_user)
                || (self.key.kind == ItemKind::MergeRequest
                    && self.reviewers.iter().any(|user| user.id == current_user)))
    }

    pub fn attention_rank(&self, current_user: u64) -> u8 {
        if self.state != "opened" {
            return 9;
        }
        if self.pipeline.as_ref().is_some_and(|p| p.status == "failed") {
            return 0;
        }
        if self.unresolved.is_some_and(|n| n > 0) {
            return 1;
        }
        if self.key.kind == ItemKind::MergeRequest
            && self
                .reviewers
                .iter()
                .any(|user| user.id == current_user && current_user != 0)
        {
            return 2;
        }
        if self.needs_reviewers(current_user) {
            return 3;
        }
        if self.is_merge_request()
            && !self.draft
            && self
                .pipeline
                .as_ref()
                .is_some_and(|p| p.status == "success")
        {
            return 4;
        }
        if self.is_merge_request()
            && self.pipeline.as_ref().is_some_and(|p| {
                matches!(
                    p.status.as_str(),
                    "created" | "pending" | "preparing" | "running" | "waiting_for_resource"
                )
            })
        {
            return 5;
        }
        if self.is_merge_request() && self.draft {
            return 6;
        }
        if self.is_merge_request() { 7 } else { 8 }
    }

    pub fn attention_reasons(&self, current_user: u64) -> Vec<String> {
        let mut reasons = Vec::new();
        if self.pipeline.as_ref().is_some_and(|p| p.status == "failed") {
            reasons.push("Pipeline failed".into());
        }
        if let Some(count) = self.unresolved.filter(|count| *count > 0) {
            reasons.push(format!("{count} unresolved threads"));
        }
        if self.key.kind == ItemKind::MergeRequest
            && self
                .reviewers
                .iter()
                .any(|user| user.id == current_user && current_user != 0)
        {
            reasons.push("Review assigned".into());
        }
        if self.needs_reviewers(current_user) {
            reasons.push("Reviewers needed".into());
        }
        if self.is_merge_request()
            && !self.draft
            && self
                .pipeline
                .as_ref()
                .is_some_and(|p| p.status == "success")
        {
            reasons.push("Checks passed".into());
        }
        if self.is_merge_request()
            && self.pipeline.as_ref().is_some_and(|p| {
                matches!(
                    p.status.as_str(),
                    "created" | "pending" | "preparing" | "running" | "waiting_for_resource"
                )
            })
        {
            reasons.push("Checks in progress".into());
        }
        if self.is_merge_request() && self.draft {
            reasons.push("Draft".into());
        }
        if reasons.is_empty() {
            reasons.push(if self.is_merge_request() {
                "Open merge request".into()
            } else {
                "Open issue".into()
            });
        }
        reasons
    }

    fn is_merge_request(&self) -> bool {
        self.key.kind == ItemKind::MergeRequest
    }

    fn needs_reviewers(&self, current_user: u64) -> bool {
        self.is_merge_request()
            && !self.draft
            && self.reviewers.is_empty()
            && current_user != 0
            && (self.author.id == current_user
                || self.assignees.iter().any(|user| user.id == current_user))
    }
}

#[cfg(test)]
mod dashboard_tests {
    use super::*;

    const ME: u64 = 1;

    #[test]
    fn user_profiles_deserialize_optional_public_email_and_bot_flag() {
        let profile: User = serde_json::from_value(serde_json::json!({
            "id": 7,
            "username": "opaque-bot-123",
            "name": "Deploy token",
            "public_email": "deploy@example.org",
            "bot": true
        }))
        .unwrap();
        assert_eq!(profile.public_email.as_deref(), Some("deploy@example.org"));
        assert!(profile.bot);

        let sparse: User = serde_json::from_value(serde_json::json!({
            "id": 8,
            "username": "human"
        }))
        .unwrap();
        assert_eq!(sparse.public_email, None);
        assert!(!sparse.bot);
    }

    fn mr() -> WorkItem {
        WorkItem {
            key: ItemKey {
                project: 1,
                iid: 1,
                kind: ItemKind::MergeRequest,
            },
            state: "opened".into(),
            author: User {
                id: ME,
                ..User::default()
            },
            ..WorkItem::default()
        }
    }

    #[test]
    fn dashboard_reasons_and_roles_are_distinct() {
        let mut item = mr();
        item.assignees.push(User {
            id: ME,
            ..User::default()
        });
        item.reviewers.push(User {
            id: ME,
            ..User::default()
        });
        item.unresolved = Some(2);
        item.pipeline = Some(Pipeline {
            status: "failed".into(),
            ..Pipeline::default()
        });

        assert_eq!(item.dashboard_roles(ME), ["author", "assignee", "reviewer"]);
        assert_eq!(
            item.attention_reasons(ME),
            ["Pipeline failed", "2 unresolved threads", "Review assigned"]
        );
    }

    #[test]
    fn urgency_puts_actionable_signals_before_waiting_and_drafts() {
        let mut failed = mr();
        failed.pipeline = Some(Pipeline {
            status: "failed".into(),
            ..Pipeline::default()
        });
        let mut discussions = mr();
        discussions.unresolved = Some(2);
        let mut review = mr();
        review.reviewers.push(User {
            id: ME,
            ..User::default()
        });
        let needs_reviewers = mr();
        let mut running = mr();
        running.reviewers.push(User {
            id: 2,
            ..User::default()
        });
        running.pipeline = Some(Pipeline {
            status: "running".into(),
            ..Pipeline::default()
        });
        let mut draft = mr();
        draft.draft = true;
        let issue = WorkItem {
            key: ItemKey {
                kind: ItemKind::Issue,
                ..mr().key
            },
            ..mr()
        };

        assert!(failed.attention_rank(ME) < discussions.attention_rank(ME));
        assert!(discussions.attention_rank(ME) < review.attention_rank(ME));
        assert!(review.attention_rank(ME) < needs_reviewers.attention_rank(ME));
        assert!(needs_reviewers.attention_rank(ME) < running.attention_rank(ME));
        assert!(running.attention_rank(ME) < draft.attention_rank(ME));
        assert!(draft.attention_rank(ME) < issue.attention_rank(ME));
        assert_eq!(needs_reviewers.attention_reasons(ME), ["Reviewers needed"]);
        assert_eq!(running.attention_reasons(ME), ["Checks in progress"]);
        assert_eq!(draft.attention_reasons(ME), ["Draft"]);
    }

    #[test]
    fn missing_discussion_count_is_unknown_and_missing_identity_does_not_match() {
        let mut item = mr();
        item.unresolved = None;
        item.author.id = 2;
        assert!(!item.dashboard_roles(0).contains(&"author"));
        assert!(!item.on_dashboard_for(0));
        assert_eq!(item.attention_reasons(ME), ["Open merge request"]);
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Note {
    pub id: u64,
    pub author: User,
    pub body: String,
    pub created_at: String,
    pub system: bool,
    pub resolvable: bool,
    pub resolved: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Discussion {
    pub id: String,
    pub notes: Vec<Note>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub stage: String,
    pub status: String,
    pub allow_failure: bool,
    pub web_url: String,
}

impl Job {
    pub fn running(&self) -> bool {
        self.status == "running"
    }
    pub fn retryable(&self) -> bool {
        matches!(self.status.as_str(), "failed" | "canceled" | "success")
    }
    /// Failed in a way that fails the pipeline (not an allowed failure).
    pub fn failed_hard(&self) -> bool {
        self.status == "failed" && !self.allow_failure
    }
    /// Status used for colouring: allowed failures are a warning, not an error.
    pub fn display_status(&self) -> &str {
        if self.status == "failed" && self.allow_failure {
            STATUS_FAILED_ALLOWED
        } else {
            &self.status
        }
    }
}

/// Pseudo-statuses only used for colouring and tooltips. The text before ` (` is the label to show.
pub const STATUS_FAILED_ALLOWED: &str = "failed (allowed)";

/// The text to display for a (pseudo-)status.
pub fn status_label(status: &str) -> &str {
    status.split(" (").next().unwrap_or(status)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Diff {
    pub old_path: String,
    pub new_path: String,
    pub diff: String,
    pub new_file: bool,
    pub deleted_file: bool,
    pub too_large: bool,
    pub collapsed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DetailPart {
    Core,
    Activity,
    Discussions,
    Pipeline,
    Changes,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Details {
    pub item: WorkItem,
    pub notes: Vec<Note>,
    pub discussions: Vec<Discussion>,
    pub jobs: Vec<Job>,
    /// Fork pipelines can run in a different project from the merge request.
    pub jobs_project: Option<u64>,
    pub diffs: Vec<Diff>,
    pub warnings: Vec<String>,
    /// Sections with usable content. Empty sections are distinct from unfetched ones.
    pub loaded: std::collections::HashSet<DetailPart>,
}

#[derive(Clone, Debug)]
pub struct TraceChunk {
    pub text: String,
    pub next_offset: u64,
    pub reset: bool,
}

#[derive(Clone, Debug)]
pub enum Mutation {
    Edit {
        key: ItemKey,
        field: String,
        value: String,
    },
    Comment {
        key: ItemKey,
        body: String,
    },
    Reply {
        key: ItemKey,
        discussion: String,
        body: String,
    },
    Resolve {
        key: ItemKey,
        discussion: String,
        resolved: bool,
    },
    RetryJob {
        project: u64,
        job: u64,
    },
    CreateIssue {
        project: u64,
        title: String,
        description: String,
    },
    CreateMergeRequest {
        project: u64,
        title: String,
        source: String,
        target: String,
        description: String,
    },
}
