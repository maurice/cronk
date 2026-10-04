use serde::{Deserialize, Serialize};

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LookupKind {
    Users,
    Milestones,
    Iterations,
    Projects,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookupOption {
    pub id: u64,
    pub label: String,
    pub value: String,
    pub description: String,
}

impl LookupOption {
    pub fn user(user: &User) -> Self {
        Self {
            id: user.id,
            label: user.name.clone(),
            value: format!("@{}", user.username),
            description: format!("@{} · ID {}", user.username, user.id),
        }
    }
    pub fn project(project: &Project) -> Self {
        Self {
            id: project.id,
            label: project.name().to_owned(),
            value: project.path.clone(),
            description: format!("{} · ID {}", project.path, project.id),
        }
    }
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
    pub fn attention_rank(&self) -> u8 {
        if self.state != "opened" {
            return 5;
        }
        if self.pipeline.as_ref().is_some_and(|p| p.status == "failed") {
            return 0;
        }
        if self.unresolved.is_some_and(|n| n > 0) {
            return 1;
        }
        if !self.draft
            && self
                .pipeline
                .as_ref()
                .is_some_and(|p| p.status == "success")
        {
            return 2;
        }
        if self.key.kind == ItemKind::MergeRequest {
            3
        } else {
            4
        }
    }
    pub fn attention_reason(&self) -> &'static str {
        match self.attention_rank() {
            0 => "Pipeline failed",
            1 => "Unresolved discussions",
            2 => "Pipeline passed · review candidate",
            3 => "In progress",
            _ => "Assigned work",
        }
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

#[derive(Clone, Debug, Default)]
pub struct Details {
    pub item: WorkItem,
    pub notes: Vec<Note>,
    pub discussions: Vec<Discussion>,
    pub jobs: Vec<Job>,
    /// Fork pipelines can run in a different project from the merge request.
    pub jobs_project: Option<u64>,
    pub diffs: Vec<Diff>,
    pub warnings: Vec<String>,
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
